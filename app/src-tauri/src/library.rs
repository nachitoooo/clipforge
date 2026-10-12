//! Clips library: list, rename and delete the files in the clips folder — the
//! `.mp4` clips and the `.png` screenshots.
//!
//! Pure filesystem logic, free of Tauri types so it is unit testable. The UI
//! only ever sends file *names*; every name is validated here so a compromised
//! webview can't touch files outside the clips folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::folders::FolderStore;

const CLIP_EXTENSION: &str = "mp4";
const SCREENSHOT_EXTENSION: &str = "png";
const MAX_STEM_LEN: usize = 120;
const FORBIDDEN_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// What a file in the library is. Everything the editor does (trim, waveform)
/// is for clips only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    /// A recorded video, `.mp4`.
    Clip,
    /// A picture of the screen, `.png`.
    Screenshot,
}

impl ClipKind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Clip => CLIP_EXTENSION,
            Self::Screenshot => SCREENSHOT_EXTENSION,
        }
    }

    /// The kind of file `path` is by its extension (any capitals), if the
    /// library holds that sort of file at all.
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        [Self::Clip, Self::Screenshot]
            .into_iter()
            .find(|kind| extension.eq_ignore_ascii_case(kind.extension()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipInfo {
    /// File name inside the clips folder; the UI's stable id.
    pub name: String,
    pub kind: ClipKind,
    /// Absolute path, for the webview asset protocol.
    pub path: String,
    pub size_bytes: u64,
    /// Milliseconds since the Unix epoch.
    pub modified_ms: u64,
    /// Ids of the library folders the clip is filed in (see `folders.rs`).
    /// `list_clips` leaves it empty; the command fills it in.
    pub folders: Vec<String>,
    /// The game (application) the clip was recorded in, if known. Like
    /// `folders`, filled in by the command from `clipmeta.rs`.
    pub game: Option<String>,
}

/// All clips and screenshots in `dir`, newest first. A missing folder is an
/// empty library.
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
            let kind = ClipKind::of(&path)?;
            let meta = entry.metadata().ok().filter(|m| m.is_file())?;
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            Some(ClipInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                kind,
                path: path.to_string_lossy().into_owned(),
                size_bytes: meta.len(),
                modified_ms,
                folders: Vec::new(),
                game: None,
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

/// A bare file name: no folders in it, nothing that walks out of the library.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', '\0']) && name != "." && name != ".."
}

/// Resolve a UI-supplied name to a path inside `dir`, for a **clip** — a video:
/// what the editor opens. A screenshot's name is refused here.
pub fn clip_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    if is_plain_name(name) && ClipKind::of(Path::new(name)) == Some(ClipKind::Clip) {
        Ok(dir.join(name))
    } else {
        Err(format!("nombre de clip inválido: {name:?}"))
    }
}

/// Resolve a UI-supplied name to a path inside `dir`, for anything the library
/// holds, clip or screenshot: what rename, delete, folders and copy work on.
pub fn media_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    if is_plain_name(name) && ClipKind::of(Path::new(name)).is_some() {
        Ok(dir.join(name))
    } else {
        Err(format!("nombre de clip inválido: {name:?}"))
    }
}

pub fn delete_clip(dir: &Path, name: &str) -> Result<(), String> {
    let path = media_path(dir, name)?;
    fs::remove_file(&path).map_err(|e| format!("no se pudo borrar el archivo: {e}"))
}

/// Delete a library folder. With `delete_clips`, the clips that are in this
/// folder and in no other one are deleted from the disk too (the ones that
/// are also filed elsewhere stay: they just leave this folder); without it the
/// clips are untouched. Returns the names of the files that were deleted, so
/// whatever else knows those clips can forget them. If a file can't be deleted
/// the folder is kept, so nothing is left half done silently.
pub fn delete_folder(
    dir: &Path,
    store: &FolderStore,
    id: &str,
    delete_clips: bool,
) -> Result<Vec<String>, String> {
    let mut deleted = Vec::new();
    if delete_clips {
        let mut gone = Vec::new();
        let mut failed = 0;
        for name in store.exclusive_clips(id)? {
            let path = media_path(dir, &name)?;
            match fs::remove_file(&path) {
                Ok(()) => {
                    deleted.push(name.clone());
                    gone.push(name);
                }
                // Already deleted behind our back: nothing left to delete.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => gone.push(name),
                Err(_) => failed += 1,
            }
        }
        store.forget_clips(&gone)?;
        if failed > 0 {
            return Err(format!(
                "no se pudieron borrar {failed} clips; la carpeta se conservó"
            ));
        }
    }
    store.delete(id)?;
    Ok(deleted)
}

/// Validate a user-typed name (with or without a `.mp4` / `.png` suffix) and
/// return the bare stem. The file keeps its own extension: a screenshot stays a
/// `.png` whatever is typed.
pub fn sanitize_stem(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let stem = [CLIP_EXTENSION, SCREENSHOT_EXTENSION]
        .iter()
        .find_map(|extension| {
            let cut = trimmed.len().checked_sub(extension.len() + 1)?;
            let (stem, suffix) = (trimmed.get(..cut)?, trimmed.get(cut..)?);
            let dotted = suffix.starts_with('.') && suffix[1..].eq_ignore_ascii_case(extension);
            dotted.then_some(stem)
        })
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

/// Rename a clip or a screenshot; returns the new file name, with the same
/// extension. Never overwrites another file.
pub fn rename_clip(dir: &Path, name: &str, new_name: &str) -> Result<String, String> {
    let from = media_path(dir, name)?;
    let kind = ClipKind::of(&from).ok_or_else(|| format!("nombre de clip inválido: {name:?}"))?;
    let new_file = format!("{}.{}", sanitize_stem(new_name)?, kind.extension());
    if new_file == name {
        return Ok(new_file);
    }
    let to = dir.join(&new_file);
    if to.exists() {
        return Err("ya existe un archivo con ese nombre".into());
    }
    fs::rename(&from, &to).map_err(|e| format!("no se pudo renombrar el archivo: {e}"))?;
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

    // ---- deleting a library folder ------------------------------------------

    fn store_in(dir: &TempDir) -> FolderStore {
        FolderStore::load(dir.0.join("config").join("library.json"))
    }

    fn folder(store: &FolderStore, name: &str, clips: &[&str]) -> String {
        let id = store
            .create(&crate::folders::FolderDraft {
                name: name.into(),
                color: "blue".into(),
                icon: "folder".into(),
            })
            .unwrap();
        let names: Vec<String> = clips.iter().map(|c| c.to_string()).collect();
        store.add_to_folder(&id, &names).unwrap();
        id
    }

    #[test]
    fn deleting_a_folder_can_keep_its_clips() {
        let dir = TempDir::new("del-folder-keep");
        let store = store_in(&dir);
        let path = dir.touch("a.mp4", 1, 1);
        let id = folder(&store, "Mía", &["a.mp4"]);

        assert!(delete_folder(&dir.0, &store, &id, false)
            .unwrap()
            .is_empty());
        assert!(path.exists());
        assert!(store.list(&list_clips(&dir.0).unwrap()).is_empty());
    }

    #[test]
    fn deleting_a_folder_with_its_clips_spares_the_shared_ones() {
        let dir = TempDir::new("del-folder-all");
        let store = store_in(&dir);
        let only = dir.touch("only.mp4", 1, 1);
        let shared = dir.touch("shared.mp4", 1, 1);
        let elsewhere = dir.touch("elsewhere.mp4", 1, 1);
        let id = folder(&store, "Borrar", &["only.mp4", "shared.mp4"]);
        let other = folder(&store, "Otra", &["shared.mp4", "elsewhere.mp4"]);

        assert_eq!(
            delete_folder(&dir.0, &store, &id, true).unwrap(),
            ["only.mp4"]
        );
        assert!(!only.exists());
        assert!(shared.exists() && elsewhere.exists());

        let mut clips = list_clips(&dir.0).unwrap();
        store.annotate(&mut clips);
        let in_other: Vec<_> = clips
            .iter()
            .filter(|c| c.folders == [other.clone()])
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(in_other.len(), 2, "the other folder lost nothing");
    }

    #[test]
    fn a_clip_already_gone_does_not_block_deleting_the_folder() {
        let dir = TempDir::new("del-folder-dangling");
        let store = store_in(&dir);
        let real = dir.touch("real.mp4", 1, 1);
        let id = folder(&store, "Con fantasma", &["real.mp4", "ghost.mp4"]);

        assert_eq!(
            delete_folder(&dir.0, &store, &id, true).unwrap(),
            ["real.mp4"]
        );
        assert!(!real.exists());
        assert!(store.list(&[]).is_empty());
    }

    #[test]
    fn if_a_clip_cannot_be_deleted_the_folder_stays() {
        let dir = TempDir::new("del-folder-fail");
        let store = store_in(&dir);
        let fine = dir.touch("fine.mp4", 1, 1);
        // A directory named like a clip: remove_file refuses it.
        fs::create_dir(dir.0.join("stuck.mp4")).unwrap();
        let id = folder(&store, "Atascada", &["fine.mp4", "stuck.mp4"]);

        let err = delete_folder(&dir.0, &store, &id, true).unwrap_err();
        assert!(err.contains("1 clips"), "{err}");
        assert!(!fine.exists(), "the deletable one is gone");
        let listed = store.list(&[ClipInfo {
            name: "stuck.mp4".into(),
            kind: ClipKind::Clip,
            path: String::new(),
            size_bytes: 0,
            modified_ms: 0,
            folders: Vec::new(),
            game: None,
        }]);
        assert_eq!(listed.len(), 1, "the folder is still there");
        assert_eq!(listed[0].clip_count, 1, "holding only what is left");
    }

    #[test]
    fn deleting_an_unknown_folder_is_an_error() {
        let dir = TempDir::new("del-folder-unknown");
        let store = store_in(&dir);
        assert!(delete_folder(&dir.0, &store, "f99", false).is_err());
        assert!(delete_folder(&dir.0, &store, "f99", true).is_err());
    }

    #[test]
    fn listed_clips_start_without_folders() {
        let dir = TempDir::new("list-no-folders");
        dir.touch("a.mp4", 1, 1);
        assert!(list_clips(&dir.0).unwrap()[0].folders.is_empty());
    }

    // ---- screenshots ----

    #[test]
    fn screenshots_are_listed_with_the_clips_and_know_what_they_are() {
        let dir = TempDir::new("screenshots-list");
        dir.touch("clip-1.mp4", 100, 30);
        dir.touch("captura-1.png", 10, 10);
        dir.touch("UPPER.PNG", 20, 20);
        dir.touch("photo.jpg", 5, 1);
        dir.touch("captura-2.png.part", 5, 1); // a capture in progress is not listed

        let listed = list_clips(&dir.0).unwrap();
        let seen: Vec<_> = listed.iter().map(|c| (c.name.as_str(), c.kind)).collect();
        assert_eq!(
            seen,
            [
                ("captura-1.png", ClipKind::Screenshot),
                ("UPPER.PNG", ClipKind::Screenshot),
                ("clip-1.mp4", ClipKind::Clip),
            ]
        );
    }

    #[test]
    fn the_ui_sees_the_kind_as_a_lowercase_word() {
        let dir = TempDir::new("screenshots-json");
        dir.touch("a.mp4", 1, 1);
        dir.touch("b.png", 1, 2);
        let json = serde_json::to_value(list_clips(&dir.0).unwrap()).unwrap();
        assert_eq!(json[0]["kind"], "clip");
        assert_eq!(json[1]["kind"], "screenshot");
    }

    #[test]
    fn the_editor_only_gets_clips_but_everything_else_gets_both() {
        let dir = Path::new("/clips");
        assert!(
            clip_path(dir, "captura-1.png").is_err(),
            "a picture can't be trimmed"
        );
        assert_eq!(
            media_path(dir, "captura-1.png").unwrap(),
            dir.join("captura-1.png")
        );
        assert_eq!(media_path(dir, "clip.MP4").unwrap(), dir.join("clip.MP4"));
        for bad in [
            "",
            "..",
            "../a.png",
            "sub/a.png",
            "a\\b.png",
            "a.txt",
            "a.png.part",
            "nul\0.png",
        ] {
            assert!(media_path(dir, bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn a_screenshot_keeps_its_extension_when_renamed() {
        let dir = TempDir::new("screenshots-rename");
        dir.touch("captura-1.png", 7, 1);
        assert_eq!(
            rename_clip(&dir.0, "captura-1.png", "Mi mejor momento").unwrap(),
            "Mi mejor momento.png"
        );
        // Typing the extension doesn't double it, and an mp4 typed on a picture
        // doesn't turn it into one.
        assert_eq!(
            rename_clip(&dir.0, "Mi mejor momento.png", "final.PNG").unwrap(),
            "final.png"
        );
        assert_eq!(
            rename_clip(&dir.0, "final.png", "final.mp4").unwrap(),
            "final.png",
            "same file name: nothing to do"
        );
        assert!(dir.0.join("final.png").exists());
    }

    #[test]
    fn a_clip_and_a_screenshot_with_the_same_stem_do_not_clash() {
        let dir = TempDir::new("screenshots-stem");
        dir.touch("momento.mp4", 1, 1);
        dir.touch("captura-1.png", 2, 1);
        assert_eq!(
            rename_clip(&dir.0, "captura-1.png", "momento").unwrap(),
            "momento.png"
        );
        assert!(dir.0.join("momento.mp4").exists() && dir.0.join("momento.png").exists());
    }

    #[test]
    fn a_screenshot_can_be_deleted_and_so_can_it_go_with_its_folder() {
        let dir = TempDir::new("screenshots-delete");
        let store = store_in(&dir);
        let picture = dir.touch("captura-1.png", 1, 1);
        let id = folder(&store, "Capturas", &["captura-1.png"]);
        assert_eq!(
            delete_folder(&dir.0, &store, &id, true).unwrap(),
            ["captura-1.png"]
        );
        assert!(!picture.exists());

        let other = dir.touch("captura-2.png", 1, 1);
        delete_clip(&dir.0, "captura-2.png").unwrap();
        assert!(!other.exists());
    }

    #[test]
    fn stems_lose_a_png_suffix_too() {
        assert_eq!(sanitize_stem("momento.png").unwrap(), "momento");
        assert_eq!(sanitize_stem("momento.PNG").unwrap(), "momento");
        assert_eq!(sanitize_stem("momento.jpg").unwrap(), "momento.jpg");
        assert!(sanitize_stem(".png").is_err());
        assert_eq!(
            sanitize_stem("ñ.mp4").unwrap(),
            "ñ",
            "multi-byte names don't confuse the cut"
        );
    }
}
