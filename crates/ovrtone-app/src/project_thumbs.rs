//! The pictures of projects that the Project Chooser shows: a small copy of the window, taken when
//! a project is opened or saved. They are kept in the settings folder and not in the project file,
//! so that the file format stays as it is and a missing picture costs nothing.
use std::path::{Path, PathBuf};

/// How big a picture is kept: enough for a tile on a high-resolution screen.
const WIDTH: usize = 336;
const HEIGHT: usize = 210;
/// A kept picture: this, the two sizes as 32-bit numbers, then 4 bytes a pixel.
const MAGIC: &[u8; 4] = b"PWT1";

fn file_for(project: &Path) -> Option<PathBuf> {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in project.to_string_lossy().bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    Some(
        crate::recent_files::config_dir()?
            .join("project-pictures")
            .join(format!("{hash:016x}")),
    )
}

/// Asks for a picture of the window for `project`.
pub fn request(ctx: &egui::Context, project: PathBuf) {
    ctx.request_screenshot(move |image| {
        // Off the UI thread: shrinking a whole window takes a moment.
        let _ = std::thread::Builder::new()
            .name("ovrtone-project-picture".into())
            .spawn(move || {
                let Some(file) = file_for(&project) else {
                    return;
                };
                let small = shrink(&image, WIDTH, HEIGHT);
                let saved = file
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&file, encode(&small, WIDTH, HEIGHT)));
                if let Err(e) = saved {
                    tracing::warn!("could not keep the project's picture: {e}");
                }
            });
    });
}

/// `image` at the size asked for, each pixel the average of what it covers. The picture is cut to
/// the shape of a tile from the top left, where the tracks are, not squeezed.
fn shrink(image: &egui::ColorImage, width: usize, height: usize) -> Vec<u8> {
    let [from_w, from_h] = image.size;
    let mut out = Vec::with_capacity(width * height * 4);
    if from_w == 0 || from_h == 0 {
        out.resize(width * height * 4, 0);
        return out;
    }
    let scale = (from_w as f32 / width as f32).min(from_h as f32 / height as f32);
    for y in 0..height {
        for x in 0..width {
            let (x0, y0) = ((x as f32 * scale) as usize, (y as f32 * scale) as usize);
            let (x1, y1) = (
                (((x + 1) as f32 * scale).ceil() as usize).clamp(x0 + 1, from_w.max(x0 + 1)),
                (((y + 1) as f32 * scale).ceil() as usize).clamp(y0 + 1, from_h.max(y0 + 1)),
            );
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for py in y0..y1.min(from_h) {
                for px in x0..x1.min(from_w) {
                    let [r, g, b, _] = image.pixels[py * from_w + px].to_array();
                    sum[0] += u32::from(r);
                    sum[1] += u32::from(g);
                    sum[2] += u32::from(b);
                    count += 1;
                }
            }
            let count = count.max(1);
            out.extend_from_slice(&[
                (sum[0] / count) as u8,
                (sum[1] / count) as u8,
                (sum[2] / count) as u8,
                255,
            ]);
        }
    }
    out
}

fn encode(pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + pixels.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(width as u32).to_le_bytes());
    out.extend_from_slice(&(height as u32).to_le_bytes());
    out.extend_from_slice(pixels);
    out
}

/// The picture in `bytes`, if it is whole.
fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (width, rest) = rest.split_first_chunk::<4>()?;
    let (height, pixels) = rest.split_first_chunk::<4>()?;
    let (width, height) = (
        u32::from_le_bytes(*width) as usize,
        u32::from_le_bytes(*height) as usize,
    );
    (pixels.len() == width.checked_mul(height)?.checked_mul(4)?)
        .then(|| egui::ColorImage::from_rgba_unmultiplied([width, height], pixels))
}

/// The project's picture, if it has one. Read once for each time the picture was taken.
pub fn texture(ctx: &egui::Context, project: &Path) -> Option<egui::TextureHandle> {
    let file = file_for(project)?;
    let taken = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
    let id = egui::Id::unique(("project_picture", &file, taken));
    if let Some(found) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(found);
    }
    let image = decode(&std::fs::read(&file).ok()?)?;
    let texture = ctx.load_texture(
        "ovrtone-project-picture",
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_shrunk_kept_and_read_back() {
        let white = egui::Color32::WHITE;
        let image = egui::ColorImage::new([1000, 800], vec![white; 800_000]);
        let small = shrink(&image, WIDTH, HEIGHT);
        assert_eq!(small.len(), WIDTH * HEIGHT * 4);
        assert!(small.iter().all(|b| *b == 255));
        let back = decode(&encode(&small, WIDTH, HEIGHT)).unwrap();
        assert_eq!(back.size, [WIDTH, HEIGHT]);
    }

    #[test]
    fn a_small_or_empty_window_and_a_damaged_file_cost_nothing() {
        let tiny = egui::ColorImage::new([3, 2], vec![egui::Color32::RED; 6]);
        assert_eq!(shrink(&tiny, WIDTH, HEIGHT).len(), WIDTH * HEIGHT * 4);
        let none = egui::ColorImage::new([0, 0], Vec::new());
        assert_eq!(shrink(&none, WIDTH, HEIGHT).len(), WIDTH * HEIGHT * 4);
        assert!(decode(b"nonsense").is_none());
        assert!(decode(&encode(&[0; 8], WIDTH, HEIGHT)).is_none());
    }

    #[test]
    fn each_project_has_its_own_picture() {
        assert_ne!(
            file_for(Path::new("a.ovrtone")),
            file_for(Path::new("b.ovrtone"))
        );
    }
}
