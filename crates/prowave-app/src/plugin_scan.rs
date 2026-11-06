//! Finds the installed plugins without risking the program. Reading what a plugin file offers runs
//! its code, so each file is read by a copy of this program started just for it (`--scan-plugin`),
//! on a background thread. A file that crashes or hangs that copy is remembered and left alone
//! until the user asks again. Results are cached by the file's modification time.
use prowave_host::{find_plugin_files, scan_plugin_file, PluginFormat, PluginInfo};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

const SCAN_ARG: &str = "--scan-plugin";
const CACHE_FILE: &str = "plugin-cache";
/// The plugins the user does not want in the menus, one identifier a line.
const HIDDEN_FILE: &str = "plugin-hidden";
/// How long one file may take, there only to end a real hang. A shell of hundreds of plugins takes
/// a while the first time, and its result is kept after that.
const SCAN_TIMEOUT: Duration = Duration::from_secs(20 * 60);
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
        .stdin(Stdio::null())
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
        std::thread::spawn(move || {
            let files = find_plugin_files(&crate::plugin_guard::plugin_folders());
            let total = files.len();
            let Ok(exe) = std::env::current_exe() else {
                let _ = tx.send(ScanMsg::Done(Vec::new()));
                ctx.request_repaint();
                return;
            };
            let all = scan_all(
                &files,
                &cache,
                retry,
                |path| scan_in_child(&exe, path, SCAN_TIMEOUT),
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
        while let Ok(msg) = rx.try_recv() {
            match msg {
                ScanMsg::Progress(n, of) => self.progress = Some((n, of)),
                ScanMsg::Done(all) => done = Some(all),
            }
        }
        if let Some(all) = done {
            self.show(&all);
            self.rx = None;
            self.progress = None;
        }
    }

    fn show(&mut self, all: &[Scanned]) {
        self.plugins = all
            .iter()
            .flat_map(|s| s.result.iter().flatten().cloned())
            .collect();
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
        let args: Vec<OsString> = ["prowave", "--scan-plugin", "a b.clap"]
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
