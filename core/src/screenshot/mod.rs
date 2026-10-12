//! A still picture of the screen, saved as a PNG next to the clips.
//!
//! * macOS: `SCScreenshotManager` (macOS 14+), the whole first display, without
//!   ClipForge's own windows in it.
//! * Windows: not yet — it waits for the Windows capture (F-2.4), which will give
//!   it one frame of Graphics Capture.
//!
//! The picture is written to a `.part` file and renamed when complete, so a
//! failure never leaves half a PNG that the library would list.

#[cfg(target_os = "macos")]
mod macos;

use std::fs;
use std::path::{Path, PathBuf};

use crate::capture::CaptureError;

/// What a screenshot looks like once saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
}

/// Where the next screenshot goes: `captura-<stamp>.png` in `dir`, with `-2`,
/// `-3`… if that name is taken (two pictures in the same second). `stamp` is
/// the local time as `YYYYMMDD-HHMMSS`.
pub fn unique_path(dir: &Path, stamp: &str) -> PathBuf {
    let base = format!("captura-{stamp}");
    let mut path = dir.join(format!("{base}.png"));
    let mut n = 2u32;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.png"));
        n += 1;
    }
    path
}

/// [`unique_path`] for right now.
pub fn next_path(dir: &Path) -> PathBuf {
    unique_path(
        dir,
        &chrono::Local::now().format("%Y%m%d-%H%M%S").to_string(),
    )
}

/// Take a picture of the screen and save it as a PNG at `path` (the folder is
/// made if it doesn't exist).
pub fn capture_png(path: &Path) -> Result<Screenshot, CaptureError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| CaptureError::Backend(format!("no se pudo crear la carpeta: {e}")))?;
    }
    let part = part_path(path);
    let result = platform_capture(&part);
    match result {
        Ok(shot) => {
            fs::rename(&part, path).map_err(|e| {
                fs::remove_file(&part).ok();
                CaptureError::Backend(format!("no se pudo guardar la captura: {e}"))
            })?;
            Ok(shot)
        }
        Err(e) => {
            fs::remove_file(&part).ok();
            Err(e)
        }
    }
}

fn part_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    path.with_file_name(name)
}

#[cfg(target_os = "macos")]
fn platform_capture(path: &Path) -> Result<Screenshot, CaptureError> {
    macos::capture_png(path)
}

#[cfg(not(target_os = "macos"))]
fn platform_capture(_path: &Path) -> Result<Screenshot, CaptureError> {
    Err(CaptureError::Backend(
        "las capturas de pantalla todavía no están disponibles en este sistema".to_string(),
    ))
}

/// The size a PNG says it has (its `IHDR` chunk), or `None` if the bytes are
/// not a PNG. Used to check what was written without decoding it.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "clipforge-shot-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn a_screenshot_is_named_after_the_time_it_was_taken() {
        let dir = Scratch::new("name");
        let path = unique_path(&dir.0, "20261012-153000");
        assert_eq!(path, dir.0.join("captura-20261012-153000.png"));
    }

    #[test]
    fn two_pictures_in_the_same_second_do_not_overwrite_each_other() {
        let dir = Scratch::new("dedupe");
        let first = unique_path(&dir.0, "20261012-153000");
        fs::write(&first, b"x").unwrap();
        let second = unique_path(&dir.0, "20261012-153000");
        assert_eq!(second, dir.0.join("captura-20261012-153000-2.png"));
        fs::write(&second, b"x").unwrap();
        assert_eq!(
            unique_path(&dir.0, "20261012-153000"),
            dir.0.join("captura-20261012-153000-3.png")
        );
        // Another second is a fresh name.
        assert_eq!(
            unique_path(&dir.0, "20261012-153001"),
            dir.0.join("captura-20261012-153001.png")
        );
    }

    #[test]
    fn the_name_of_the_next_one_is_a_png_in_the_folder() {
        let dir = Scratch::new("next");
        let path = next_path(&dir.0);
        assert_eq!(path.parent(), Some(dir.0.as_path()));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(
            name.starts_with("captura-") && name.ends_with(".png"),
            "{name}"
        );
        // captura-YYYYMMDD-HHMMSS.png
        assert_eq!(name.len(), "captura-20261012-153000.png".len(), "{name}");
    }

    #[test]
    fn the_size_of_a_png_is_read_from_its_header() {
        assert_eq!(png_dimensions(&png(3024, 1964)), Some((3024, 1964)));
        assert_eq!(png_dimensions(&png(1, 1)), Some((1, 1)));
    }

    #[test]
    fn what_is_not_a_png_has_no_size() {
        assert_eq!(png_dimensions(b""), None);
        assert_eq!(png_dimensions(b"GIF89a not a png at all......"), None);
        let mut broken = png(10, 10);
        broken[12] = b'X'; // no IHDR where it must be
        assert_eq!(png_dimensions(&broken), None);
        assert_eq!(png_dimensions(&png(10, 10)[..20]), None, "cut short");
    }

    #[test]
    fn the_temporary_file_is_never_listed_as_a_picture() {
        let part = part_path(Path::new("/clips/captura-1.png"));
        assert_eq!(part, Path::new("/clips/captura-1.png.part"));
        assert_ne!(part.extension().and_then(|e| e.to_str()), Some("png"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn without_a_capture_backend_it_says_so_and_leaves_nothing_behind() {
        let dir = Scratch::new("unsupported");
        let path = dir.0.join("captura.png");
        let err = capture_png(&path).unwrap_err().to_string();
        assert!(err.contains("todavía no están disponibles"), "{err}");
        assert!(!path.exists() && !part_path(&path).exists());
    }
}
