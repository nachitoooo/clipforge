//! Put a clip *file* on the system clipboard, so pasting in a chat, a mail or a
//! folder attaches or copies the file (the same as "Copy" in the Finder or
//! Explorer). Nothing is read back and nothing leaves the machine.
//!
//! * macOS: `NSPasteboard` with the file's URL.
//! * Windows: a `CF_HDROP` list (`DROPFILES`), compiled and linted by the CI
//!   cross-check but never run (F-2.4 is open).

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::path::Path;

/// Copy the file at `path` to the clipboard, replacing what it held. The path
/// must be absolute and name a file that exists.
pub fn copy_file(path: &Path) -> Result<(), String> {
    check(path)?;
    platform_copy(path)
}

fn check(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("la ruta del archivo no es absoluta".to_string());
    }
    if !path.is_file() {
        return Err("el clip ya no existe en el disco".to_string());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn platform_copy(path: &Path) -> Result<(), String> {
    macos::copy_file(path)
}

#[cfg(target_os = "windows")]
fn platform_copy(path: &Path) -> Result<(), String> {
    windows::copy_file(path)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_copy(_path: &Path) -> Result<(), String> {
    Err("copiar al portapapeles no está disponible en este sistema".to_string())
}

/// The `CF_HDROP` payload for one file: a `DROPFILES` header followed by the
/// path as UTF-16 and two terminating NULs (the second ends the list).
///
/// ```text
/// DROPFILES { pFiles: u32 = 20, pt: POINT { x: i32, y: i32 }, fNC: i32, fWide: i32 = 1 }
/// ```
///
/// Pure, so it is tested on every OS even though only Windows uses it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn hdrop_payload(path: &Path) -> Result<Vec<u8>, String> {
    const HEADER_BYTES: u32 = 20;
    let text = path
        .to_str()
        .ok_or_else(|| "la ruta del archivo no es texto válido".to_string())?;
    if text.contains('\0') {
        return Err("la ruta del archivo tiene un carácter no permitido".to_string());
    }
    let mut bytes = Vec::with_capacity(HEADER_BYTES as usize + (text.len() + 2) * 2);
    bytes.extend(HEADER_BYTES.to_le_bytes()); // pFiles: where the list starts
    bytes.extend(0i32.to_le_bytes()); // pt.x
    bytes.extend(0i32.to_le_bytes()); // pt.y
    bytes.extend(0i32.to_le_bytes()); // fNC
    bytes.extend(1i32.to_le_bytes()); // fWide: the names are UTF-16
    for unit in text.encode_utf16().chain([0, 0]) {
        bytes.extend(unit.to_le_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn units(bytes: &[u8]) -> Vec<u16> {
        bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn the_drop_list_has_a_dropfiles_header_then_the_wide_path() {
        let bytes = hdrop_payload(Path::new("C:\\clips\\a.mp4")).unwrap();
        assert_eq!(
            u32_at(&bytes, 0),
            20,
            "the list starts right after the header"
        );
        assert_eq!(u32_at(&bytes, 4), 0, "pt.x");
        assert_eq!(u32_at(&bytes, 8), 0, "pt.y");
        assert_eq!(u32_at(&bytes, 12), 0, "not a non-client drop");
        assert_eq!(u32_at(&bytes, 16), 1, "wide characters");

        let list = units(&bytes[20..]);
        let text: String = String::from_utf16(&list[..list.len() - 2]).unwrap();
        assert_eq!(text, "C:\\clips\\a.mp4");
        assert_eq!(
            &list[list.len() - 2..],
            [0, 0],
            "the path and the list both end in NUL"
        );
    }

    #[test]
    fn the_size_is_exact_so_the_system_reads_nothing_more() {
        let path = "C:\\a.mp4";
        let bytes = hdrop_payload(Path::new(path)).unwrap();
        assert_eq!(bytes.len(), 20 + (path.encode_utf16().count() + 2) * 2);
    }

    #[test]
    fn accents_and_characters_outside_the_basic_plane_survive() {
        // "ñ" is one UTF-16 unit; the emoji needs a surrogate pair.
        let path = "C:\\Vídeos\\clip ñandú 🎮.mp4";
        let bytes = hdrop_payload(Path::new(path)).unwrap();
        let list = units(&bytes[20..]);
        assert_eq!(String::from_utf16(&list[..list.len() - 2]).unwrap(), path);
    }

    #[test]
    fn a_path_with_a_nul_is_refused() {
        assert!(hdrop_payload(Path::new("C:\\a\0b.mp4")).is_err());
    }

    #[test]
    fn a_relative_or_missing_path_is_refused_before_touching_the_clipboard() {
        assert!(copy_file(Path::new("clip.mp4"))
            .unwrap_err()
            .contains("absoluta"));

        let missing = std::env::temp_dir().join("clipforge-clipboard-does-not-exist.mp4");
        assert!(copy_file(&missing).unwrap_err().contains("ya no existe"));
        // A folder is not a clip either.
        let dir: PathBuf = std::env::temp_dir();
        assert!(copy_file(&dir).unwrap_err().contains("ya no existe"));
    }
}
