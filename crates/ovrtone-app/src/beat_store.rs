//! File > Export > For Beat Store: what a beat store sells, in one go. An untagged WAV and an MP3 of the
//! whole beat and a ZIP of its stems, named after the beat with its tempo and key, in a folder of its
//! own under Music\Ovrtone Beat Store. When they are made, the folder opens and so does the store's
//! site (Trakroom), to drag them in. Nothing is sent anywhere by Ovrtone itself.

use crate::bounce::{self, After, BitDepth, FileFormat, Render};
use crate::model::Project;
use crate::UiState;
use std::path::{Path, PathBuf};

/// Where a seller uploads beats.
const STORE_URL: &str = "https://trakroom.com";

/// An export under way: what to do when its files are made.
pub struct Pending {
    folder: PathBuf,
    stems: PathBuf,
    base: String,
    info: String,
}

/// Starts the export of `p`.
pub fn start(p: &Project, s: &mut UiState) {
    if s.bounce.busy() || s.beat_store.is_some() {
        s.status = Some("Wait for the export under way to finish".to_string());
        return;
    }
    let end = bounce::project_end(p);
    if end <= 0.0 {
        s.status = Some("Nothing to export yet".to_string());
        return;
    }
    let key = crate::score::key_label(p.key);
    let bpm = p.tempo.round() as i32;
    let base = bounce::file_safe(&format!("{} ({bpm} BPM, {key})", p.name));
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        s.status = Some("No user folder to export to".to_string());
        return;
    };
    let folder = home.join("Music").join("Ovrtone Beat Store").join(&base);
    let stems = folder.join("stems (being made)");
    if let Err(e) = std::fs::create_dir_all(&stems) {
        s.status = Some(format!("Cannot make the folder: {e}"));
        return;
    }
    let whole = |format: FileFormat| Render {
        path: folder.join(format!("{base}.{}", format.extension())),
        format,
        from_beat: 0.0,
        to_beat: end,
        depth: BitDepth::Int24,
        out_rate: 0,
        solo: None,
        after: After::Report,
        mono: false,
        normalize: None,
    };
    let mut renders = vec![whole(FileFormat::Wav), whole(FileFormat::Mp3)];
    renders.extend(bounce::stems(p, &stems, FileFormat::Wav, BitDepth::Int24, (0.0, end)));
    let seconds = p.tempo_map().seconds_at(end);
    let mut info = format!(
        "Title: {}\r\nBPM: {bpm}\r\nKey: {key}\r\nLength: {}:{:02}\r\n",
        p.name,
        (seconds / 60.0) as u32,
        (seconds % 60.0) as u32,
    );
    if !p.engineer.trim().is_empty() {
        info.push_str(&format!("Mixed by: {}\r\n", p.engineer.trim()));
    }
    s.bounce.enqueue(renders);
    s.beat_store = Some(Pending { folder, stems, base, info });
    s.status = Some("Exporting for your beat store: WAV, MP3 and stems".to_string());
}

/// Finishes an export once its files are made: the stems into a ZIP, a note of the beat's tempo and
/// key, and the folder and the store opened. Once a frame.
pub fn poll(s: &mut UiState) {
    if s.beat_store.is_none() || s.bounce.busy() {
        return;
    }
    let Some(job) = s.beat_store.take() else { return };
    let zip = job.folder.join(format!("{} Stems.zip", job.base));
    let zipped = zip_folder(&job.stems, &zip);
    let _ = std::fs::remove_dir_all(&job.stems);
    let _ = std::fs::write(job.folder.join("Beat info.txt"), &job.info);
    s.status = Some(match zipped {
        Ok(0) => "Exported the WAV and MP3 (no tracks for stems)".to_string(),
        Ok(n) => format!("Exported for your beat store: WAV, MP3 and {n} stems. Drag them into Trakroom."),
        Err(e) => format!("Exported the WAV and MP3; the stems ZIP failed: {e}"),
    });
    let _ = std::process::Command::new("explorer").arg(&job.folder).spawn();
    let _ = std::process::Command::new("explorer").arg(STORE_URL).spawn();
}

/// Every file in `folder` into the ZIP `out`. Returns how many.
fn zip_folder(folder: &Path, out: &Path) -> Result<usize, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(0);
    }
    let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    for path in &files {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        let mut input = std::fs::File::open(path).map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut zip).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(files.len())
}
