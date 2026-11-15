//! Finds the installed plugins without risking the program. Reading what a plugin file offers runs
//! its code, so each file is read by a copy of this program started just for it (`--scan-plugin`),
//! on a background thread. A file that crashes or hangs that copy is remembered and left alone
//! until the user asks again. Results are cached by the file's modification time.
use ovrtone_host::{find_plugin_files, scan_plugin_file, PluginFormat, PluginInfo};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

const SCAN_ARG: &str = "--scan-plugin";
const CACHE_FILE: &str = "plugin-cache";
/// The plugins the user does not want in the menus, one identifier a line.
const HIDDEN_FILE: &str = "plugin-hidden";
/// How long one file may take, there only to end a real hang. A shell of hundreds of plugins takes
/// a while the first time, and its result is kept after that.
const SCAN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// How long a scanner may do no work at all. One that uses no CPU for this long is waiting for
/// something that will not come, such as a licence window nobody can see behind the launch screen.
/// A plugin that is slow but working is left to [`SCAN_TIMEOUT`].
const SCAN_STALL: Duration = Duration::from_secs(2 * 60);
/// How often a scanner's work is looked at.
const SCAN_STALL_CHECK: Duration = Duration::from_secs(5);
/// The scanner's last line: the list above it is whole, whatever happens as the process ends.
const SCAN_DONE: &str = "scan-done";
/// The first line of the cache. Files in a cache without it get another chance if they failed:
/// the scanner has since learned to wait longer and to survive a crash on the way out.
const CACHE_VERSION: &str = "version\t2";
/// Files read at the same time. A big collection takes minutes otherwise, one by one.
const SCAN_WORKERS: usize = 4;

/// The file to read when this copy was started as a scanner.
pub fn scan_request(args: &[OsString]) -> Option<PathBuf> {
    let at = args.iter().position(|a| a == SCAN_ARG)?;
    args.get(at + 1).map(PathBuf::from)
}

/// A value that fits in a tab-separated line.
fn clean(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

fn plugin_line(info: &PluginInfo) -> String {
    format!(
        "plugin\t{}\t{}\t{}\t{}\t{}",
        clean(&info.id),
        clean(&info.name),
        clean(&info.vendor),
        clean(&info.version),
        u8::from(info.is_instrument)
    )
}

fn parse_plugin_line(line: &str, path: &Path) -> Option<PluginInfo> {
    let mut parts = line.strip_prefix("plugin\t")?.split('\t');
    let (id, name, vendor, version, instrument) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    Some(PluginInfo {
        id: id.to_string(),
        name: name.to_string(),
        vendor: vendor.to_string(),
        version: version.to_string(),
        format: PluginFormat::of(path),
        path: path.to_path_buf(),
        is_instrument: instrument.trim() == "1",
    })
}

/// The scanner copy's whole job: prints what `path` offers, one plugin a line. Plugins print things
/// of their own, which the reader skips. Returns the process's exit code.
pub fn scan_and_print(path: &Path) -> i32 {
    // The program that asked keeps this pipe open. If it goes, for any reason, so does this copy:
    // a plugin that hangs here would otherwise stay running with nobody to stop it.
    std::thread::spawn(|| {
        let mut sink = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut sink);
        crate::leave_now(2);
    });
    match scan_plugin_file(path) {
        Ok(found) => {
            for info in &found {
                println!("{}", plugin_line(info));
            }
            println!("{SCAN_DONE}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Reads one file in a copy of `exe`, which is killed after `timeout`.
fn scan_in_child(exe: &Path, path: &Path, timeout: Duration) -> Result<Vec<PluginInfo>, String> {
    let mut command = Command::new(exe);
    command
        .arg(SCAN_ARG)
        .arg(path)
        // Held open while this waits, and closed by Windows if this program ends.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    run_scanner(command, path, timeout)
}

fn run_scanner(
    mut command: Command,
    path: &Path,
    timeout: Duration,
) -> Result<Vec<PluginInfo>, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start the scanner: {e}"))?;
    // Read as it comes, so a plugin that prints a lot cannot fill the pipe and look like a hang.
    let mut stdout = child.stdout.take();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(out) = stdout.as_mut() {
            let _ = out.read_to_end(&mut text);
        }
        let _ = tx.send(text);
    });
    let start = Instant::now();
    let mut work = WorkWatch::new(child.id());
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "did not finish loading in {} seconds",
                    timeout.as_secs()
                ));
            }
            Ok(None) if work.stalled() => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("stopped responding while loading".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let text = rx
        .recv_timeout(Duration::from_secs(5))
        .map(|t| String::from_utf8_lossy(&t).into_owned())
        .unwrap_or_default();
    // Some plugins crash while the process ends, with their list already out.
    let whole = text.lines().any(|l| l.trim() == SCAN_DONE);
    if !status.success() && !whole {
        return Err(match status.code() {
            Some(1) => "could not be loaded".to_string(),
            _ => "crashed while loading".to_string(),
        });
    }
    Ok(text
        .lines()
        .filter_map(|l| parse_plugin_line(l, path))
        .collect())
}

/// Sees whether a process is still doing any work.
struct WorkWatch {
    system: sysinfo::System,
    pid: sysinfo::Pid,
    /// The CPU time it had used when last it was seen to use more, and when that was.
    used: u64,
    since: Instant,
    checked: Instant,
}

impl WorkWatch {
    fn new(pid: u32) -> Self {
        let now = Instant::now();
        Self {
            system: sysinfo::System::new(),
            pid: sysinfo::Pid::from_u32(pid),
            used: 0,
            since: now,
            checked: now,
        }
    }

    /// True once it has used no CPU for [`SCAN_STALL`].
    fn stalled(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.checked) < SCAN_STALL_CHECK {
            return false;
        }
        self.checked = now;
        self.system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[self.pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_cpu(),
        );
        let Some(process) = self.system.process(self.pid) else {
            return false;
        };
        let used = process.accumulated_cpu_time();
        if used != self.used {
            self.used = used;
            self.since = now;
        }
        now.duration_since(self.since) >= SCAN_STALL
    }
}

/// What is known about one plugin file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Scanned {
    path: PathBuf,
    /// Modification time: a file that changed is read again.
    stamp: u64,
    result: Result<Vec<PluginInfo>, String>,
}

fn stamp_of(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// The cache as text: a `file` line for each file, then the lines of its plugins.
fn cache_text(all: &[Scanned]) -> String {
    let mut text = format!("{CACHE_VERSION}\n");
    for s in all {
        let failure = s.result.as_ref().err().map_or("", String::as_str);
        text.push_str(&format!(
            "file\t{}\t{}\t{}\n",
            s.path.display(),
            s.stamp,
            clean(failure)
        ));
        for info in s.result.iter().flatten() {
            text.push_str(&plugin_line(info));
            text.push('\n');
        }
    }
    text
}

fn parse_cache(text: &str) -> Vec<Scanned> {
    let mut all: Vec<Scanned> = Vec::new();
    let current = text
        .lines()
        .next()
        .is_some_and(|l| l.trim() == CACHE_VERSION);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("file\t") {
            let mut parts = rest.split('\t');
            let (Some(path), Some(stamp), failure) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(stamp) = stamp.parse() else {
                continue;
            };
            if !current && failure.is_some_and(|f| !f.is_empty()) {
                continue;
            }
            all.push(Scanned {
                path: PathBuf::from(path),
                stamp,
                result: match failure {
                    Some(f) if !f.is_empty() => Err(f.to_string()),
                    _ => Ok(Vec::new()),
                },
            });
        } else if let Some(last) = all.last_mut() {
            let path = last.path.clone();
            if let (Ok(found), Some(info)) = (&mut last.result, parse_plugin_line(line, &path)) {
                found.push(info);
            }
        }
    }
    all
}

/// The cached result for a file if it still holds. A failure is only kept when `retry` is off.
fn reuse(cache: &[Scanned], path: &Path, stamp: u64, retry: bool) -> Option<Scanned> {
    cache
        .iter()
        .find(|s| s.path == path && s.stamp == stamp && !(retry && s.result.is_err()))
        .cloned()
}

/// Brings the cache up to date with `files`, reading with `read` what it does not hold, and
/// tells `progress` after each file.
fn scan_all(
    files: &[PathBuf],
    cache: &[Scanned],
    retry: bool,
    read: impl Fn(&Path) -> Result<Vec<PluginInfo>, String> + Sync,
    progress: impl Fn(usize) + Sync,
) -> Vec<Scanned> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<Scanned>>> = files.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..SCAN_WORKERS.min(files.len()) {
            scope.spawn(|| {
                while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let stamp = stamp_of(path);
                    let scanned = reuse(cache, path, stamp, retry).unwrap_or_else(|| Scanned {
                        path: path.clone(),
                        stamp,
                        result: read(path),
                    });
                    let i = next_index(files, path);
                    if let Ok(mut slot) = results[i].lock() {
                        *slot = Some(scanned);
                    }
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1);
                }
            });
        }
    });
    results
        .into_iter()
        .filter_map(|slot| slot.into_inner().ok().flatten())
        .collect()
}

/// Where `path`, which comes from `files`, sits in it.
fn next_index(files: &[PathBuf], path: &Path) -> usize {
    files.iter().position(|f| f == path).unwrap_or_default()
}

enum ScanMsg {
    Progress(usize, usize),
    Done(Vec<Scanned>),
}

/// What tells two formats of one plugin apart from different plugins: the name, the maker's first
/// word ("FabFilter" and "FabFilter Software" are one maker), and instrument or effect.
fn same_plugin_key(p: &PluginInfo) -> (String, String, bool) {
    let plain = |text: &str| -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let maker = p.vendor.split_whitespace().next().unwrap_or_default();
    (plain(&p.name), plain(maker), p.is_instrument)
}

/// A plugin installed as both CLAP and VST3 is offered as CLAP. A project that already uses the VST3
/// goes on loading it: this is only the list to choose from.
fn prefer_clap(plugins: Vec<PluginInfo>) -> Vec<PluginInfo> {
    let clap: std::collections::HashSet<_> = plugins
        .iter()
        .filter(|p| p.format == PluginFormat::Clap)
        .map(same_plugin_key)
        .collect();
    plugins
        .into_iter()
        .filter(|p| p.format != PluginFormat::Vst3 || !clap.contains(&same_plugin_key(p)))
        .collect()
}

/// The installed CLAP plugins as far as they are known, and the scan bringing them up to date.
#[derive(Default)]
pub struct PluginScan {
    pub plugins: Vec<PluginInfo>,
    /// Files that would not load, and why.
    pub failed: Vec<(PathBuf, String)>,
    /// Files read and files to read, while a scan runs.
    pub progress: Option<(usize, usize)>,
    /// Left out of the menus, by the user's choice. They still load in a project that uses them.
    pub hidden: std::collections::HashSet<String>,
    /// The plugin file being read just now, by name; empty while only checking the known ones.
    pub reading: std::sync::Arc<std::sync::Mutex<String>>,
    rx: Option<Receiver<ScanMsg>>,
    started: bool,
}

impl PluginScan {
    /// Shows or hides a plugin in the menus, and remembers it.
    pub fn set_hidden(&mut self, id: &str, hidden: bool) {
        if hidden {
            self.hidden.insert(id.to_string());
        } else {
            self.hidden.remove(id);
        }
        let Some(file) = crate::recent_files::config_dir().map(|d| d.join(HIDDEN_FILE)) else {
            return;
        };
        let mut lines: Vec<&str> = self.hidden.iter().map(String::as_str).collect();
        lines.sort_unstable();
        let saved = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, lines.join("\n")));
        if let Err(e) = saved {
            tracing::warn!("could not save the hidden plugins: {e}");
        }
    }

    /// Starts the first scan. Anything already known comes from the cache at once.
    pub fn ensure_started(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.start(false, ctx);
        }
    }

    /// Scans again; `retry` gives files that failed before another chance.
    pub fn start(&mut self, retry: bool, ctx: &egui::Context) {
        if self.rx.is_some() {
            return;
        }
        self.started = true;
        if let Some(file) = crate::recent_files::config_dir().map(|d| d.join(HIDDEN_FILE)) {
            if let Ok(text) = std::fs::read_to_string(file) {
                self.hidden = text.lines().map(str::to_string).collect();
            }
        }
        let cache_path = crate::recent_files::config_dir().map(|d| d.join(CACHE_FILE));
        let cache = cache_path
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| parse_cache(&t))
            .unwrap_or_default();
        self.show(&cache);
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.progress = Some((0, 0));
        let ctx = ctx.clone();
        let reading = self.reading.clone();
        std::thread::spawn(move || {
            let began = Instant::now();
            let files = find_plugin_files(&crate::plugin_guard::plugin_folders());
            let total = files.len();
            let _ = tx.send(ScanMsg::Progress(0, total));
            let Ok(exe) = std::env::current_exe() else {
                let _ = tx.send(ScanMsg::Done(Vec::new()));
                ctx.request_repaint();
                return;
            };
            // What is known so far is saved after each new file, so that a long first scan cut
            // short does not start over: only what it had not reached is still to read.
            let known = Mutex::new(cache.clone());
            let all = scan_all(
                &files,
                &cache,
                retry,
                |path| {
                    if let (Some(name), Ok(mut now)) = (path.file_stem(), reading.lock()) {
                        *now = name.to_string_lossy().into_owned();
                    }
                    let result = scan_in_child(&exe, path, SCAN_TIMEOUT);
                    if let (Some(file), Ok(mut known)) = (&cache_path, known.lock()) {
                        known.retain(|s| s.path != path);
                        known.push(Scanned {
                            path: path.to_path_buf(),
                            stamp: stamp_of(path),
                            result: result.clone(),
                        });
                        let saved = file
                            .parent()
                            .map_or(Ok(()), std::fs::create_dir_all)
                            .and_then(|()| std::fs::write(file, cache_text(&known)));
                        if let Err(e) = saved {
                            tracing::warn!("could not save the plugin list: {e}");
                        }
                    }
                    result
                },
                |done| {
                    let _ = tx.send(ScanMsg::Progress(done, total));
                    ctx.request_repaint();
                },
            );
            if let Some(file) = cache_path {
                let saved = file
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&file, cache_text(&all)));
                if let Err(e) = saved {
                    tracing::warn!("could not save the plugin list: {e}");
                }
            }
            tracing::info!(
                "plugin scan: {total} files in {:.2} s",
                began.elapsed().as_secs_f32()
            );
            let _ = tx.send(ScanMsg::Done(all));
            ctx.request_repaint();
        });
    }

    /// Takes in what the scan has found so far.
    pub fn drain(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        let mut done = None;
        let mut gone = false;
        loop {
            match rx.try_recv() {
                Ok(ScanMsg::Progress(n, of)) => self.progress = Some((n, of)),
                Ok(ScanMsg::Done(all)) => done = Some(all),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    gone = true;
                    break;
                }
            }
        }
        let finished = done.is_some() || gone;
        if let Some(all) = done {
            self.show(&all);
        }
        // Also if the scan's thread ended without a word: nothing is coming, and the launch waits
        // for this.
        if finished {
            self.rx = None;
            self.progress = None;
        }
    }

    fn show(&mut self, all: &[Scanned]) {
        let found: Vec<PluginInfo> = all
            .iter()
            .flat_map(|s| s.result.iter().flatten().cloned())
            .collect();
        crate::plugin_trust::set_known(&found);
        self.plugins = prefer_clap(found);
        self.plugins
            .sort_by_key(|p| (p.name.to_lowercase(), p.id.clone()));
        self.failed = all
            .iter()
            .filter_map(|s| s.result.as_ref().err().map(|e| (s.path.clone(), e.clone())))
            .collect();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn clap_is_offered_over_vst3_of_the_same_plugin() {
        let make = |name: &str, vendor: &str, format, is_instrument| PluginInfo {
            id: format!("{name}-{format:?}"),
            name: name.to_string(),
            vendor: vendor.to_string(),
            version: String::new(),
            format,
            path: PathBuf::new(),
            is_instrument,
        };
        let (clap, vst3) = (PluginFormat::Clap, PluginFormat::Vst3);
        let kept = prefer_clap(vec![
            make("Pro-Q 3", "FabFilter", vst3, false),
            make("Pro-Q 3", "FabFilter Software", clap, false),
            // No CLAP of this one: its VST3 stays.
            make("H-Comp", "Waves", vst3, false),
            // A name shared with another maker's, or with an instrument, is another plugin.
            make("Comp", "One", clap, false),
            make("Comp", "Two", vst3, false),
            make("Comp", "One", vst3, true),
        ]);
        let names: Vec<(&str, PluginFormat)> =
            kept.iter().map(|p| (p.name.as_str(), p.format)).collect();
        assert_eq!(
            names,
            [
                ("Pro-Q 3", clap),
                ("H-Comp", vst3),
                ("Comp", clap),
                ("Comp", vst3),
                ("Comp", vst3),
            ]
        );
    }

    fn info(name: &str, is_instrument: bool, path: &Path) -> PluginInfo {
        PluginInfo {
            id: format!("com.test.{name}"),
            name: name.to_string(),
            vendor: "Maker".to_string(),
            version: "1.2".to_string(),
            format: PluginFormat::Clap,
            path: path.to_path_buf(),
            is_instrument,
        }
    }

    #[test]
    fn a_scanner_is_asked_for_by_its_argument() {
        let args: Vec<OsString> = ["ovrtone", "--scan-plugin", "a b.clap"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(scan_request(&args), Some(PathBuf::from("a b.clap")));
        assert_eq!(scan_request(&args[..2]), None, "no file named");
        assert_eq!(scan_request(&args[..1]), None);
    }

    #[test]
    fn plugin_lines_round_trip_and_what_plugins_print_is_skipped() {
        let path = PathBuf::from("x.clap");
        let synth = info("Synth", true, &path);
        assert_eq!(
            parse_plugin_line(&plugin_line(&synth), &path),
            Some(synth.clone())
        );
        let tabs = info("A\tB\nC", false, &path);
        assert_eq!(
            parse_plugin_line(&plugin_line(&tabs), &path).unwrap().name,
            "A B C"
        );
        assert_eq!(parse_plugin_line("Licence checked", &path), None);
        assert_eq!(parse_plugin_line("plugin\ttoo\tshort", &path), None);
    }

    #[test]
    fn the_cache_round_trips_with_plugins_and_failures() {
        let good = PathBuf::from("C:\\Plugins\\good.clap");
        let all = vec![
            Scanned {
                path: good.clone(),
                stamp: 42,
                result: Ok(vec![info("One", false, &good), info("Two", true, &good)]),
            },
            Scanned {
                path: PathBuf::from("empty.clap"),
                stamp: 1,
                result: Ok(Vec::new()),
            },
            Scanned {
                path: PathBuf::from("bad.clap"),
                stamp: 7,
                result: Err("crashed while loading".to_string()),
            },
        ];
        assert_eq!(parse_cache(&cache_text(&all)), all);
        assert!(parse_cache("nonsense\nfile\tonly-a-path\n").is_empty());
    }

    #[test]
    fn only_new_or_changed_files_are_read_and_failures_wait_for_a_retry() {
        let dir = tempfile::tempdir().unwrap();
        let (good, bad, new) = (
            dir.path().join("good.clap"),
            dir.path().join("bad.clap"),
            dir.path().join("new.clap"),
        );
        for f in [&good, &bad, &new] {
            std::fs::write(f, b"").unwrap();
        }
        let cache = vec![
            Scanned {
                path: good.clone(),
                stamp: stamp_of(&good),
                result: Ok(vec![info("Good", false, &good)]),
            },
            Scanned {
                path: bad.clone(),
                stamp: stamp_of(&bad),
                result: Err("crashed while loading".to_string()),
            },
        ];
        let files = [good.clone(), bad.clone(), new.clone()];
        let reads = AtomicUsize::new(0);
        let read = |p: &Path| {
            reads.fetch_add(1, Ordering::Relaxed);
            Ok(vec![info("Read", false, p)])
        };
        let last = AtomicUsize::new(0);
        let all = scan_all(&files, &cache, false, read, |n| {
            last.fetch_max(n, Ordering::Relaxed);
        });
        assert_eq!(reads.load(Ordering::Relaxed), 1, "only the new file");
        assert_eq!(last.load(Ordering::Relaxed), 3);
        assert_eq!(all[0].path, good, "in the order of the files");
        assert!(all[1].result.is_err(), "a crash is not tried again");
        reads.store(0, Ordering::Relaxed);
        scan_all(&files, &cache, true, read, |_| {});
        assert_eq!(
            reads.load(Ordering::Relaxed),
            2,
            "a retry reads the failure too"
        );
        reads.store(0, Ordering::Relaxed);
        let changed = vec![Scanned {
            stamp: 1,
            ..cache[0].clone()
        }];
        scan_all(&files[..1], &changed, false, read, |_| {});
        assert_eq!(
            reads.load(Ordering::Relaxed),
            1,
            "a changed file is read again"
        );
    }

    #[test]
    fn the_list_is_sorted_and_failures_are_kept_apart() {
        let a = PathBuf::from("a.clap");
        let mut scan = PluginScan::default();
        scan.show(&[
            Scanned {
                path: a.clone(),
                stamp: 0,
                result: Ok(vec![info("zeta", false, &a), info("Alpha", false, &a)]),
            },
            Scanned {
                path: PathBuf::from("b.clap"),
                stamp: 0,
                result: Err("crashed while loading".to_string()),
            },
        ]);
        let names: Vec<_> = scan.plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta"]);
        assert_eq!(scan.failed.len(), 1);
    }

    /// A scanner that hangs is stopped, one that fails is reported, and neither stops the host.
    #[cfg(windows)]
    #[test]
    fn a_hung_or_failing_scanner_is_an_error() {
        let path = Path::new("x.clap");
        let mut hang = Command::new("ping");
        hang.args(["-n", "30", "127.0.0.1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let start = Instant::now();
        let err = run_scanner(hang, path, Duration::from_millis(150)).unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(10), "it was killed");

        let mut fail = Command::new("cmd");
        fail.args(["/c", "exit 1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        assert_eq!(
            run_scanner(fail, path, SCAN_TIMEOUT).unwrap_err(),
            "could not be loaded"
        );

        let mut crash = Command::new("cmd");
        crash
            .args(["/c", "exit -1073741819"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        assert_eq!(
            run_scanner(crash, path, SCAN_TIMEOUT).unwrap_err(),
            "crashed while loading"
        );

        let mut crash_at_the_end = Command::new("cmd");
        crash_at_the_end
            .args([
                "/c",
                "echo plugin\ta\tA\tMaker\t1\t0& echo scan-done& exit -1073741819",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let kept = run_scanner(crash_at_the_end, path, SCAN_TIMEOUT).unwrap();
        assert_eq!(kept.len(), 1, "the list was whole before the crash");

        let old = parse_cache("file\tx\t1\tcrashed while loading\nfile\ty\t2\t\n");
        assert_eq!(old.len(), 1, "an old failure gets another chance");
        let new = parse_cache(&format!(
            "{CACHE_VERSION}\nfile\tx\t1\tcrashed while loading\n"
        ));
        assert!(new[0].result.is_err(), "a failure since then is remembered");

        let mut ok = Command::new("cmd");
        ok.args(["/c", "echo noise& echo plugin\ta\tA\tMaker\t1\t0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let found = run_scanner(ok, path, SCAN_TIMEOUT).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "A");
    }
}
