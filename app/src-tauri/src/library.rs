//! Clips library: list, rename and delete the .mp4 files in the clips folder.
//!
//! Pure filesystem logic, free of Tauri types so it is unit testable. The UI
//! only ever sends file *names*; every name is validated here so a compromised
//! webview can't touch files outside the clips folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

const EXTENSION: &str = "mp4";
const MAX_STEM_LEN: usize = 120;
const FORBIDDEN_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipInfo {
    /// File name inside the clips folder; the UI's stable id.
    pub name: String,
    /// Absolute path, for the webview asset protocol.
    pub path: String,
    pub size_bytes: u64,
    /// Milliseconds since the Unix epoch.
    pub modified_ms: u64,
}

fn is_clip_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// All clips in `dir`, newest first. A missing folder is an empty library.
pub fn list_clips(dir: &Path) -> Result<Vec<ClipInfo>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("no se pudo leer la carpeta de clips: {e}")),
    };

    let mut clips: Vec<ClipInfo> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !is_clip_file(&path) {
                return None;
            }
            let meta = entry.metadata().ok().filter(|m| m.is_file())?;
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            Some(ClipInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
                size_bytes: meta.len(),
                modified_ms,
            })
        })
        .collect();

    clips.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    Ok(clips)
}

/// Resolve a UI-supplied clip file name to a path inside `dir`.
pub fn clip_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let is_plain_name = !name.is_empty()
        && !name.contains(['/', '\\', '\0'])
        && name != "."
        && name != ".."
        && is_clip_file(Path::new(name));
    if !is_plain_name {
        return Err(format!("nombre de clip inválido: {name:?}"));
    }
    Ok(dir.join(name))
}

pub fn delete_clip(dir: &Path, name: &str) -> Result<(), String> {
    let path = clip_path(dir, name)?;
    fs::remove_file(&path).map_err(|e| format!("no se pudo borrar el clip: {e}"))
}

/// Validate a user-typed clip name (with or without the `.mp4` suffix) and
/// return the bare stem.
pub fn sanitize_stem(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let stem = trimmed
        .strip_suffix(".mp4")
        .or_else(|| trimmed.strip_suffix(".MP4"))
        .unwrap_or(trimmed)
        .trim();

    if stem.is_empty() {
        return Err("el nombre no puede estar vacío".into());
    }
    if stem.chars().count() > MAX_STEM_LEN {
        return Err(format!(
            "el nombre no puede superar {MAX_STEM_LEN} caracteres"
        ));
    }
    if stem.starts_with('.') {
        return Err("el nombre no puede empezar con un punto".into());
    }
    if stem
        .chars()
        .any(|c| c.is_control() || FORBIDDEN_CHARS.contains(&c))
    {
        return Err("el nombre contiene caracteres no permitidos".into());
    }
    Ok(stem.to_string())
}

/// Rename a clip; returns the new file name. Never overwrites another clip.
pub fn rename_clip(dir: &Path, name: &str, new_name: &str) -> Result<String, String> {
    let from = clip_path(dir, name)?;
    let new_file = format!("{}.{EXTENSION}", sanitize_stem(new_name)?);
    if new_file == name {
        return Ok(new_file);
    }
    let to = dir.join(&new_file);
    if to.exists() {
        return Err("ya existe un clip con ese nombre".into());
    }
    fs::rename(&from, &to).map_err(|e| format!("no se pudo renombrar el clip: {e}"))?;
    Ok(new_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    /// Scratch directory removed on drop (no extra dev-dependency needed).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "clipforge-library-{label}-{}",
                crate::testutil::unique()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn touch(&self, name: &str, bytes: usize, age_secs: u64) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, vec![0u8; bytes]).unwrap();
            let when = SystemTime::now() - Duration::from_secs(age_secs);
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn missing_folder_is_an_empty_library() {
        let dir = TempDir::new("missing");
        let gone = dir.0.join("nope");
        assert_eq!(list_clips(&gone).unwrap(), Vec::new());
    }

    #[test]
    fn empty_folder_lists_nothing() {
        let dir = TempDir::new("empty");
        assert!(list_clips(&dir.0).unwrap().is_empty());
    }

    #[test]
    fn lists_only_mp4_files_newest_first() {
        let dir = TempDir::new("list");
        dir.touch("old.mp4", 10, 300);
        dir.touch("new.mp4", 20, 5);
        dir.touch("UPPER.MP4", 30, 60);
        dir.touch("notes.txt", 5, 1);
        fs::create_dir(dir.0.join("sub.mp4")).unwrap();

        let clips = list_clips(&dir.0).unwrap();
        let names: Vec<_> = clips.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["new.mp4", "UPPER.MP4", "old.mp4"]);
        assert_eq!(clips[0].size_bytes, 20);
        assert!(clips[0].modified_ms > clips[2].modified_ms);
        assert!(Path::new(&clips[0].path).is_absolute());
    }

    #[test]
    fn equal_timestamps_sort_by_name_descending() {
        let dir = TempDir::new("ties");
        let a = dir.touch("clip-a.mp4", 1, 10);
        let b = dir.touch("clip-b.mp4", 1, 10);
        let when = SystemTime::now() - Duration::from_secs(10);
        for p in [&a, &b] {
            fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        let names: Vec<_> = list_clips(&dir.0)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["clip-b.mp4", "clip-a.mp4"]);
    }

    #[test]
    fn clip_path_rejects_traversal_and_non_clips() {
        let dir = Path::new("/clips");
        for bad in [
            "",
            ".",
            "..",
            "../escape.mp4",
            "sub/clip.mp4",
            "sub\\clip.mp4",
            "/etc/passwd",
            "clip.txt",
            "clip",
            "nul\0.mp4",
        ] {
            assert!(clip_path(dir, bad).is_err(), "{bad:?} should be rejected");
        }
        assert_eq!(
            clip_path(dir, "clip-1.mp4").unwrap(),
            dir.join("clip-1.mp4")
        );
    }

    #[test]
    fn delete_removes_the_file_and_rejects_unknown_ones() {
        let dir = TempDir::new("delete");
        let path = dir.touch("gone.mp4", 1, 1);
        delete_clip(&dir.0, "gone.mp4").unwrap();
        assert!(!path.exists());
        assert!(delete_clip(&dir.0, "gone.mp4").is_err());
        assert!(delete_clip(&dir.0, "../gone.mp4").is_err());
    }

    #[test]
    fn sanitize_stem_accepts_good_names() {
        assert_eq!(sanitize_stem("  Ace en Mirage ").unwrap(), "Ace en Mirage");
        assert_eq!(sanitize_stem("clutch.mp4").unwrap(), "clutch");
        assert_eq!(sanitize_stem("clutch.MP4").unwrap(), "clutch");
        assert_eq!(sanitize_stem("año nuevo ñandú").unwrap(), "año nuevo ñandú");
        assert_eq!(
            sanitize_stem(&"a".repeat(MAX_STEM_LEN)).unwrap().len(),
            MAX_STEM_LEN
        );
    }

    #[test]
    fn sanitize_stem_rejects_bad_names() {
        for bad in [
            "", "   ", ".mp4", ".hidden", "a/b", "a\\b", "a:b", "a*b", "a?b",
        ] {
            assert!(sanitize_stem(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(sanitize_stem("tab\there").is_err());
        assert!(sanitize_stem(&"a".repeat(MAX_STEM_LEN + 1)).is_err());
    }

    #[test]
    fn rename_moves_the_file_and_returns_the_new_name() {
        let dir = TempDir::new("rename");
        dir.touch("clip-1.mp4", 7, 1);
        let new = rename_clip(&dir.0, "clip-1.mp4", "Mi mejor jugada").unwrap();
        assert_eq!(new, "Mi mejor jugada.mp4");
        assert!(!dir.0.join("clip-1.mp4").exists());
        assert_eq!(fs::metadata(dir.0.join(&new)).unwrap().len(), 7);
    }

    #[test]
    fn rename_to_same_name_is_a_noop() {
        let dir = TempDir::new("rename-same");
        dir.touch("clip-1.mp4", 1, 1);
        assert_eq!(
            rename_clip(&dir.0, "clip-1.mp4", "clip-1").unwrap(),
            "clip-1.mp4"
        );
        assert!(dir.0.join("clip-1.mp4").exists());
    }

    #[test]
    fn rename_never_overwrites_another_clip() {
        let dir = TempDir::new("rename-clash");
        dir.touch("a.mp4", 1, 1);
        dir.touch("b.mp4", 2, 1);
        let err = rename_clip(&dir.0, "a.mp4", "b").unwrap_err();
        assert!(err.contains("ya existe"));
        assert_eq!(fs::metadata(dir.0.join("b.mp4")).unwrap().len(), 2);
        assert!(dir.0.join("a.mp4").exists());
    }

    #[test]
    fn rename_rejects_invalid_source_and_target() {
        let dir = TempDir::new("rename-invalid");
        dir.touch("a.mp4", 1, 1);
        assert!(rename_clip(&dir.0, "../a.mp4", "b").is_err());
        assert!(rename_clip(&dir.0, "a.mp4", "../b").is_err());
        assert!(rename_clip(&dir.0, "missing.mp4", "b").is_err());
    }
}
