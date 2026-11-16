//! Help > Send Feedback…: the user writes what happened, and Ovrtone keeps it as a report with the
//! version and system details (and, if ticked, the end of the log). Until the website takes
//! reports ([`FEEDBACK_URL`] is `None`), reports wait in `%APPDATA%\ovrtone\feedback`, and the window
//! offers to copy one or open its folder to send by hand. Nothing leaves the computer by itself.

use crate::UiState;

/// Where reports go once the website takes them. `None` for now: reports wait on this computer.
pub const FEEDBACK_URL: Option<&str> = None;

/// The window, while it is open.
#[derive(Default)]
pub struct FeedbackView {
    pub open: bool,
    kind: usize,
    text: String,
    with_log: bool,
    /// The report last kept, for Copy.
    last: Option<String>,
}

const KINDS: [&str; 3] = ["Something went wrong", "An idea", "Something else"];

fn folder() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("feedback"))
}

/// The report: what the user wrote, then what helps to make sense of it. No file paths or names
/// beyond what the user wrote.
fn report(kind: &str, text: &str, with_log: bool) -> String {
    let mut out = format!(
        "Kind: {kind}\r\nOvrtone {}\r\nSystem: {} {}\r\n\r\n{}\r\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        text.trim()
    );
    if with_log {
        if let Some(crash) = last_crash_report() {
            out.push_str("\r\n--- the last crash report ---\r\n");
            out.push_str(&crash);
        }
    }
    out
}

/// The newest crash report release builds leave in the temp folder (`report-*.toml`), if any.
fn last_crash_report() -> Option<String> {
    let newest = std::fs::read_dir(std::env::temp_dir())
        .ok()?
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("report-") && name.ends_with(".toml")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?;
    let text = std::fs::read_to_string(newest.path()).ok()?;
    // A report is small; keep it so.
    Some(text.chars().take(20_000).collect())
}

/// Keeps a report in the feedback folder. Returns its file.
fn keep(text: &str) -> Result<std::path::PathBuf, String> {
    let dir = folder().ok_or("no settings folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let file = dir.join(format!("report-{stamp}.txt"));
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file)
}

/// How many reports wait to be sent.
fn waiting() -> usize {
    folder()
        .and_then(|d| std::fs::read_dir(d).ok())
        .map_or(0, |it| it.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "txt")).count())
}

pub fn window(ctx: &egui::Context, s: &mut UiState) {
    if !s.feedback.open {
        return;
    }
    let mut open = true;
    let mut message = None;
    egui::Window::new("Send Feedback")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            let v = &mut s.feedback;
            ui.horizontal(|ui| {
                for (k, name) in KINDS.iter().enumerate() {
                    ui.selectable_value(&mut v.kind, k, *name);
                }
            });
            ui.add(
                egui::TextEdit::multiline(&mut v.text)
                    .hint_text("What did you do, what did you expect, and what happened?")
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            );
            ui.checkbox(&mut v.with_log, "Add the last crash report, if there is one");
            ui.weak(match FEEDBACK_URL {
                Some(_) => "Reports are sent when you press Send.",
                None => "Sending from Ovrtone comes with the website. For now the report is kept on this \
                         computer: copy it, or open its folder, to send it.",
            });
            ui.horizontal(|ui| {
                let can = !v.text.trim().is_empty();
                if ui.add_enabled(can, egui::Button::new("Keep Report")).clicked() {
                    let text = report(KINDS[v.kind], &v.text, v.with_log);
                    match keep(&text) {
                        Ok(_) => {
                            v.last = Some(text);
                            v.text.clear();
                            message = Some("Report kept. Copy it or open its folder to send it.".to_string());
                        }
                        Err(e) => message = Some(format!("Could not keep the report: {e}")),
                    }
                }
                if ui.add_enabled(v.last.is_some(), egui::Button::new("Copy Report")).clicked() {
                    if let Some(text) = &v.last {
                        ui.ctx().copy_text(text.clone());
                        message = Some("Report copied.".to_string());
                    }
                }
                if ui.button("Open Folder").clicked() {
                    if let Some(dir) = folder() {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                    }
                }
                let n = waiting();
                if n > 0 {
                    ui.weak(format!("{n} waiting"));
                }
            });
        });
    if let Some(m) = message {
        s.status = Some(m);
    }
    if !open {
        s.feedback.open = false;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_report_says_what_and_which_version() {
        let r = super::report("An idea", "  more knobs  ", false);
        assert!(r.contains("Kind: An idea"));
        assert!(r.contains(env!("CARGO_PKG_VERSION")));
        assert!(r.contains("\r\nmore knobs\r\n"));
    }
}
