//! Library folders: groups of clips the user makes in the Biblioteca.
//!
//! A folder is a label, not a directory. The clips stay where they are in the
//! (flat) clips folder and a folder lists clip *file names*, so one clip can
//! sit in several folders at once and filing it never touches the file. The
//! index lives in `library.json` next to `settings.json`.
//!
//! Rules worth knowing:
//! * Reads never write and never prune: a name whose file is missing (deleted
//!   in the Finder, an unplugged drive) is inert, not forgotten, so a
//!   transient problem can't empty the index. Only app actions that know a
//!   clip is gone (`delete_clip`, `rename_clip`, deleting a folder with its
//!   clips) update it.
//! * Every change is all or nothing: the in-memory index only changes if the
//!   file was written.
//! * Free of Tauri types so it is unit tested; the commands live in `lib.rs`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::library::ClipInfo;
use crate::persist::JsonStore;

pub const INDEX_VERSION: u32 = 1;
pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_FOLDERS: usize = 200;
pub const DEFAULT_COLOR: &str = "orange";
pub const DEFAULT_ICON: &str = "folder";
/// The icon of a folder made automatically for a game.
pub const AUTO_ICON: &str = "gamepad";
/// The colors those folders take, one per game, so they are easy to tell apart.
/// Every id is in the UI's palette (`FOLDER_COLORS` in `lib/folders.ts`); an id
/// the UI doesn't know would only be drawn in the default color.
const AUTO_COLORS: [&str; 8] = [
    "orange", "red", "pink", "purple", "blue", "teal", "green", "yellow",
];
const MAX_STYLE_CHARS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Folder {
    /// `f1`, `f2`…: stable, never reused, survives renames.
    id: String,
    name: String,
    /// A palette id (`orange`, `blue`…); the UI knows the actual colors.
    color: String,
    /// An icon id (`folder`, `star`…); the UI knows the drawings.
    icon: String,
    /// Clip file names.
    clips: Vec<String>,
    created_ms: u64,
}

impl Default for Folder {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            color: DEFAULT_COLOR.to_string(),
            icon: DEFAULT_ICON.to_string(),
            clips: Vec::new(),
            created_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Index {
    version: u32,
    next_id: u64,
    folders: Vec<Folder>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            next_id: 1,
            folders: Vec::new(),
        }
    }
}

fn style_ok(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_STYLE_CHARS
        && value.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

impl Index {
    /// Tidy a file somebody edited by hand, so nothing downstream has to
    /// cope with duplicate ids, empty names or an id counter that went back.
    fn repaired(mut self) -> Self {
        let mut seen = HashSet::new();
        self.folders
            .retain(|f| !f.id.is_empty() && seen.insert(f.id.clone()));
        for folder in &mut self.folders {
            folder.name = folder.name.trim().chars().take(MAX_NAME_CHARS).collect();
            if folder.name.is_empty() {
                folder.name = format!("Carpeta {}", folder.id);
            }
            if !style_ok(&folder.color) {
                folder.color = DEFAULT_COLOR.to_string();
            }
            if !style_ok(&folder.icon) {
                folder.icon = DEFAULT_ICON.to_string();
            }
            let mut names = HashSet::new();
            folder
                .clips
                .retain(|c| !c.is_empty() && names.insert(c.clone()));
        }
        let highest = self
            .folders
            .iter()
            .filter_map(|f| f.id.strip_prefix('f')?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(highest + 1);
        self.version = INDEX_VERSION;
        self
    }

    fn folder_mut(&mut self, id: &str) -> Result<&mut Folder, String> {
        self.folders
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or_else(|| "la carpeta ya no existe".to_string())
    }

    /// Validate a user-typed folder name; `except` is the folder being
    /// renamed (it may keep its own name, even with different capitals).
    fn checked_name(&self, raw: &str, except: Option<&str>) -> Result<String, String> {
        let name = raw.trim();
        if name.is_empty() {
            return Err("el nombre no puede estar vacío".into());
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(format!(
                "el nombre no puede superar {MAX_NAME_CHARS} caracteres"
            ));
        }
        if name.chars().any(char::is_control) {
            return Err("el nombre contiene caracteres no permitidos".into());
        }
        let wanted = name.to_lowercase();
        let taken = self
            .folders
            .iter()
            .any(|f| Some(f.id.as_str()) != except && f.name.to_lowercase() == wanted);
        if taken {
            return Err("ya existe una carpeta con ese nombre".into());
        }
        Ok(name.to_string())
    }
}

fn checked_style(value: &str, what: &str) -> Result<String, String> {
    if style_ok(value) {
        Ok(value.to_string())
    } else {
        Err(format!("{what} no válido"))
    }
}

/// What the webview sends to create or edit a folder.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDraft {
    pub name: String,
    pub color: String,
    pub icon: String,
}

/// What the webview shows about a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInfo {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    /// Clips that exist on disk right now.
    pub clip_count: usize,
    pub size_bytes: u64,
    pub created_ms: u64,
}

/// A color for the folder of `game`. The same name gives the same color on every
/// run and platform, whatever its capitals (FNV-1a, not the std hasher, which
/// is free to change).
fn auto_color(game: &str) -> &'static str {
    let hash = game
        .to_lowercase()
        .bytes()
        .fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
    AUTO_COLORS[hash as usize % AUTO_COLORS.len()]
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Loads, holds and saves the folder index.
pub struct FolderStore {
    store: JsonStore<Index>,
}

impl FolderStore {
    /// Never fails: a missing file means no folders; an unreadable one is
    /// moved aside to `library.json.bak` (so nothing is lost) and starts empty.
    pub fn load(path: PathBuf) -> Self {
        Self {
            store: JsonStore::load(path, "la biblioteca", Index::repaired),
        }
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut Index) -> Result<T, String>) -> Result<T, String> {
        self.store.mutate(change)
    }

    fn read<T>(&self, view: impl FnOnce(&Index) -> T) -> T {
        self.store.read(view)
    }

    // ---- folders ----------------------------------------------------------

    /// Create a folder; returns its id.
    pub fn create(&self, draft: &FolderDraft) -> Result<String, String> {
        self.mutate(|index| {
            if index.folders.len() >= MAX_FOLDERS {
                return Err(format!("no se pueden tener más de {MAX_FOLDERS} carpetas"));
            }
            let folder = Folder {
                id: format!("f{}", index.next_id),
                name: index.checked_name(&draft.name, None)?,
                color: checked_style(&draft.color, "el color")?,
                icon: checked_style(&draft.icon, "el ícono")?,
                clips: Vec::new(),
                created_ms: now_ms(),
            };
            index.next_id += 1;
            let id = folder.id.clone();
            index.folders.push(folder);
            Ok(id)
        })
    }

    /// Change a folder's name, color and icon; its clips stay.
    pub fn update(&self, id: &str, draft: &FolderDraft) -> Result<(), String> {
        self.mutate(|index| {
            let name = index.checked_name(&draft.name, Some(id))?;
            let color = checked_style(&draft.color, "el color")?;
            let icon = checked_style(&draft.icon, "el ícono")?;
            let folder = index.folder_mut(id)?;
            folder.name = name;
            folder.color = color;
            folder.icon = icon;
            Ok(())
        })
    }

    /// Remove the folder from the index. The clips are never touched here.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.mutate(|index| {
            index.folder_mut(id)?;
            index.folders.retain(|f| f.id != id);
            Ok(())
        })
    }

    // ---- membership -------------------------------------------------------

    /// Automatic filing: put `clip` in the folder called `game` (a folder of the
    /// same name in other capitals counts), making that folder first if it
    /// doesn't exist yet. A folder the user already has is used as it is: its
    /// name, color and icon are never touched. All or nothing, like any change.
    pub fn file_under_game(&self, game: &str, clip: &str) -> Result<(), String> {
        self.mutate(|index| {
            let wanted = game.trim().to_lowercase();
            let at = match index
                .folders
                .iter()
                .position(|f| f.name.to_lowercase() == wanted)
            {
                Some(at) => at,
                None => {
                    if index.folders.len() >= MAX_FOLDERS {
                        return Err(format!("no se pueden tener más de {MAX_FOLDERS} carpetas"));
                    }
                    let name = index.checked_name(game, None)?;
                    let color = auto_color(&name).to_string();
                    index.folders.push(Folder {
                        id: format!("f{}", index.next_id),
                        name,
                        color,
                        icon: AUTO_ICON.to_string(),
                        clips: Vec::new(),
                        created_ms: now_ms(),
                    });
                    index.next_id += 1;
                    index.folders.len() - 1
                }
            };
            let folder = &mut index.folders[at];
            if !folder.clips.iter().any(|c| c == clip) {
                folder.clips.push(clip.to_string());
            }
            Ok(())
        })
    }

    /// Make `name` belong to exactly the folders `ids`.
    pub fn set_clip_folders(&self, name: &str, ids: &[String]) -> Result<(), String> {
        self.mutate(|index| {
            for id in ids {
                index.folder_mut(id)?;
            }
            for folder in &mut index.folders {
                let wanted = ids.contains(&folder.id);
                let present = folder.clips.iter().any(|c| c == name);
                if wanted && !present {
                    folder.clips.push(name.to_string());
                } else if !wanted && present {
                    folder.clips.retain(|c| c != name);
                }
            }
            Ok(())
        })
    }

    /// File `names` in a folder; returns how many were not already there.
    pub fn add_to_folder(&self, id: &str, names: &[String]) -> Result<usize, String> {
        self.mutate(|index| {
            let folder = index.folder_mut(id)?;
            let mut added = 0;
            for name in names {
                if !folder.clips.contains(name) {
                    folder.clips.push(name.clone());
                    added += 1;
                }
            }
            Ok(added)
        })
    }

    /// Take `names` out of a folder; returns how many were in it.
    pub fn remove_from_folder(&self, id: &str, names: &[String]) -> Result<usize, String> {
        self.mutate(|index| {
            let folder = index.folder_mut(id)?;
            let before = folder.clips.len();
            folder.clips.retain(|c| !names.contains(c));
            Ok(before - folder.clips.len())
        })
    }

    /// The clips are gone: take them out of every folder.
    pub fn forget_clips(&self, names: &[String]) -> Result<(), String> {
        self.mutate(|index| {
            for folder in &mut index.folders {
                folder.clips.retain(|c| !names.contains(c));
            }
            Ok(())
        })
    }

    /// A clip was renamed: its folders follow it.
    pub fn rename_clip(&self, old: &str, new: &str) -> Result<(), String> {
        self.mutate(|index| {
            for folder in &mut index.folders {
                let Some(at) = folder.clips.iter().position(|c| c == old) else {
                    continue;
                };
                if folder.clips.iter().any(|c| c == new) {
                    folder.clips.remove(at);
                } else {
                    folder.clips[at] = new.to_string();
                }
            }
            Ok(())
        })
    }

    /// `new` is a copy of `source` (a trimmed clip): same folders.
    pub fn inherit(&self, source: &str, new: &str) -> Result<(), String> {
        self.mutate(|index| {
            for folder in &mut index.folders {
                if folder.clips.iter().any(|c| c == source)
                    && !folder.clips.iter().any(|c| c == new)
                {
                    folder.clips.push(new.to_string());
                }
            }
            Ok(())
        })
    }

    /// Clips that are in this folder and in no other one.
    pub fn exclusive_clips(&self, id: &str) -> Result<Vec<String>, String> {
        self.read(|index| {
            let folder = index
                .folders
                .iter()
                .find(|f| f.id == id)
                .ok_or_else(|| "la carpeta ya no existe".to_string())?;
            Ok(folder
                .clips
                .iter()
                .filter(|name| {
                    !index
                        .folders
                        .iter()
                        .any(|f| f.id != id && f.clips.contains(name))
                })
                .cloned()
                .collect())
        })
    }

    // ---- reading ----------------------------------------------------------

    /// Every folder with the clips of `clips` it holds. Names without a file
    /// in `clips` are ignored (and kept: see the module docs).
    pub fn list(&self, clips: &[ClipInfo]) -> Vec<FolderInfo> {
        let sizes: HashMap<&str, u64> = clips
            .iter()
            .map(|c| (c.name.as_str(), c.size_bytes))
            .collect();
        self.read(|index| {
            index
                .folders
                .iter()
                .map(|folder| {
                    let present: Vec<u64> = folder
                        .clips
                        .iter()
                        .filter_map(|name| sizes.get(name.as_str()).copied())
                        .collect();
                    FolderInfo {
                        id: folder.id.clone(),
                        name: folder.name.clone(),
                        color: folder.color.clone(),
                        icon: folder.icon.clone(),
                        clip_count: present.len(),
                        size_bytes: present.iter().sum(),
                        created_ms: folder.created_ms,
                    }
                })
                .collect()
        })
    }

    /// Fill in which folders each clip is in.
    pub fn annotate(&self, clips: &mut [ClipInfo]) {
        self.read(|index| {
            let mut membership: HashMap<&str, Vec<String>> = HashMap::new();
            for folder in &index.folders {
                for name in &folder.clips {
                    membership
                        .entry(name.as_str())
                        .or_default()
                        .push(folder.id.clone());
                }
            }
            for clip in clips {
                clip.folders = membership.remove(clip.name.as_str()).unwrap_or_default();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::fs;

    fn store(dir: &TempDir) -> FolderStore {
        FolderStore::load(dir.0.join("library.json"))
    }

    fn draft(name: &str) -> FolderDraft {
        FolderDraft {
            name: name.into(),
            color: "blue".into(),
            icon: "star".into(),
        }
    }

    fn clip(name: &str, size: u64) -> ClipInfo {
        ClipInfo {
            name: name.into(),
            kind: crate::library::ClipKind::Clip,
            path: format!("/clips/{name}"),
            size_bytes: size,
            modified_ms: 0,
            folders: Vec::new(),
            game: None,
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn make(store: &FolderStore, name: &str) -> String {
        store.create(&draft(name)).unwrap()
    }

    #[test]
    fn ids_grow_and_are_never_reused() {
        let dir = TempDir::new("folders-ids");
        let store = store(&dir);
        assert_eq!(make(&store, "Uno"), "f1");
        let two = make(&store, "Dos");
        assert_eq!(two, "f2");
        store.delete(&two).unwrap();
        assert_eq!(make(&store, "Tres"), "f3");
    }

    #[test]
    fn names_are_trimmed_and_checked() {
        let dir = TempDir::new("folders-names");
        let store = store(&dir);
        for bad in ["", "   ", "tab\there", "new\nline"] {
            assert!(store.create(&draft(bad)).is_err(), "{bad:?}");
        }
        assert!(store
            .create(&draft(&"a".repeat(MAX_NAME_CHARS + 1)))
            .is_err());
        assert!(store.create(&draft(&"ñ".repeat(MAX_NAME_CHARS))).is_ok());
        // A label, not a file name: these are fine.
        let id = store.create(&draft("  Ranked / noche: 2  ")).unwrap();
        assert_eq!(store.list(&[])[1].name, "Ranked / noche: 2");
        assert_eq!(id, "f2");
    }

    #[test]
    fn two_folders_cannot_share_a_name_even_with_other_capitals() {
        let dir = TempDir::new("folders-dupes");
        let store = store(&dir);
        let valorant = make(&store, "Valorant");
        assert!(store
            .create(&draft("valorant"))
            .unwrap_err()
            .contains("ya existe"));
        let other = make(&store, "Otra");
        assert!(store.update(&other, &draft("VALORANT")).is_err());
        // A folder may change the capitals of its own name.
        store.update(&valorant, &draft("VALORANT")).unwrap();
        assert_eq!(store.list(&[])[0].name, "VALORANT");
    }

    #[test]
    fn there_is_a_folder_limit() {
        let dir = TempDir::new("folders-limit");
        let store = store(&dir);
        for n in 0..MAX_FOLDERS {
            make(&store, &format!("Carpeta {n}"));
        }
        assert!(store
            .create(&draft("Una más"))
            .unwrap_err()
            .contains("más de"));
    }

    #[test]
    fn color_and_icon_must_be_short_ids() {
        let dir = TempDir::new("folders-style");
        let store = store(&dir);
        for bad in [
            "",
            "Red",
            "red!",
            "has space",
            "x".repeat(17).as_str(),
            "../x",
        ] {
            let mut d = draft("Estilo");
            d.color = bad.into();
            assert!(store.create(&d).is_err(), "color {bad:?}");
            let mut d = draft("Estilo");
            d.icon = bad.into();
            assert!(store.create(&d).is_err(), "icon {bad:?}");
        }
        assert!(store.list(&[]).is_empty());
    }

    #[test]
    fn editing_a_folder_keeps_its_clips() {
        let dir = TempDir::new("folders-edit");
        let store = store(&dir);
        let id = make(&store, "Antes");
        store.add_to_folder(&id, &names(&["a.mp4"])).unwrap();
        store
            .update(
                &id,
                &FolderDraft {
                    name: "Después".into(),
                    color: "green".into(),
                    icon: "trophy".into(),
                },
            )
            .unwrap();
        let info = &store.list(&[clip("a.mp4", 5)])[0];
        assert_eq!(
            (info.name.as_str(), info.color.as_str(), info.icon.as_str()),
            ("Después", "green", "trophy")
        );
        assert_eq!(info.clip_count, 1);
        assert!(store.update("f99", &draft("x")).is_err());
    }

    #[test]
    fn a_clip_can_be_in_several_folders() {
        let dir = TempDir::new("folders-many");
        let store = store(&dir);
        let a = make(&store, "A");
        let b = make(&store, "B");
        store
            .add_to_folder(&a, &names(&["x.mp4", "y.mp4"]))
            .unwrap();
        store.add_to_folder(&b, &names(&["x.mp4"])).unwrap();

        let mut clips = vec![clip("x.mp4", 1), clip("y.mp4", 1), clip("z.mp4", 1)];
        store.annotate(&mut clips);
        assert_eq!(clips[0].folders, [a.clone(), b.clone()]);
        assert_eq!(clips[1].folders, [a.as_str()]);
        assert!(clips[2].folders.is_empty());
    }

    #[test]
    fn set_clip_folders_replaces_the_membership() {
        let dir = TempDir::new("folders-set");
        let store = store(&dir);
        let a = make(&store, "A");
        let b = make(&store, "B");
        let c = make(&store, "C");
        store
            .set_clip_folders("x.mp4", &[a.clone(), b.clone()])
            .unwrap();
        store
            .set_clip_folders("x.mp4", &[b.clone(), c.clone()])
            .unwrap();

        let mut clips = vec![clip("x.mp4", 1)];
        store.annotate(&mut clips);
        assert_eq!(clips[0].folders, [b, c]);

        store.set_clip_folders("x.mp4", &[]).unwrap();
        store.annotate(&mut clips);
        assert!(clips[0].folders.is_empty());
    }

    #[test]
    fn set_clip_folders_with_an_unknown_folder_changes_nothing() {
        let dir = TempDir::new("folders-set-unknown");
        let store = store(&dir);
        let a = make(&store, "A");
        let err = store
            .set_clip_folders("x.mp4", &[a.clone(), "f99".into()])
            .unwrap_err();
        assert!(err.contains("ya no existe"));
        assert_eq!(store.list(&[clip("x.mp4", 1)])[0].clip_count, 0);
    }

    #[test]
    fn add_and_remove_report_how_many_changed() {
        let dir = TempDir::new("folders-addrm");
        let store = store(&dir);
        let a = make(&store, "A");
        assert_eq!(
            store
                .add_to_folder(&a, &names(&["x.mp4", "y.mp4", "x.mp4"]))
                .unwrap(),
            2
        );
        assert_eq!(store.add_to_folder(&a, &names(&["x.mp4"])).unwrap(), 0);
        assert_eq!(
            store
                .remove_from_folder(&a, &names(&["x.mp4", "nope.mp4"]))
                .unwrap(),
            1
        );
        assert_eq!(store.remove_from_folder(&a, &names(&["x.mp4"])).unwrap(), 0);
        assert!(store.add_to_folder("f99", &names(&["x.mp4"])).is_err());
        assert!(store.remove_from_folder("f99", &names(&["x.mp4"])).is_err());
    }

    #[test]
    fn a_renamed_clip_keeps_its_folders() {
        let dir = TempDir::new("folders-rename-clip");
        let store = store(&dir);
        let a = make(&store, "A");
        let b = make(&store, "B");
        store
            .add_to_folder(&a, &names(&["old.mp4", "other.mp4"]))
            .unwrap();
        store.add_to_folder(&b, &names(&["old.mp4"])).unwrap();
        store.rename_clip("old.mp4", "new.mp4").unwrap();

        let mut clips = vec![clip("old.mp4", 1), clip("new.mp4", 1), clip("other.mp4", 1)];
        store.annotate(&mut clips);
        assert!(clips[0].folders.is_empty());
        assert_eq!(clips[1].folders, [a.clone(), b]);
        assert_eq!(clips[2].folders, [a]);
    }

    #[test]
    fn a_deleted_clip_leaves_every_folder() {
        let dir = TempDir::new("folders-forget");
        let store = store(&dir);
        let a = make(&store, "A");
        let b = make(&store, "B");
        store
            .add_to_folder(&a, &names(&["x.mp4", "keep.mp4"]))
            .unwrap();
        store.add_to_folder(&b, &names(&["x.mp4"])).unwrap();
        store.forget_clips(&names(&["x.mp4"])).unwrap();

        let clips = [clip("x.mp4", 1), clip("keep.mp4", 1)];
        let counts: Vec<_> = store.list(&clips).iter().map(|f| f.clip_count).collect();
        assert_eq!(counts, [1, 0]);
    }

    #[test]
    fn a_trimmed_copy_joins_the_folders_of_the_original() {
        let dir = TempDir::new("folders-inherit");
        let store = store(&dir);
        let a = make(&store, "A");
        let b = make(&store, "B");
        store.add_to_folder(&a, &names(&["clip.mp4"])).unwrap();
        store.inherit("clip.mp4", "clip-recorte.mp4").unwrap();
        store.inherit("clip.mp4", "clip-recorte.mp4").unwrap(); // twice: still once

        let mut clips = vec![clip("clip.mp4", 1), clip("clip-recorte.mp4", 1)];
        store.annotate(&mut clips);
        assert_eq!(clips[0].folders, [a.as_str()]);
        assert_eq!(clips[1].folders, [a]);
        assert_eq!(store.list(&clips)[1].id, b);
        assert_eq!(store.list(&clips)[1].clip_count, 0);
    }

    #[test]
    fn exclusive_clips_are_the_ones_in_no_other_folder() {
        let dir = TempDir::new("folders-exclusive");
        let store = store(&dir);
        let a = make(&store, "A");
        let b = make(&store, "B");
        store
            .add_to_folder(&a, &names(&["only-a.mp4", "both.mp4"]))
            .unwrap();
        store.add_to_folder(&b, &names(&["both.mp4"])).unwrap();
        assert_eq!(store.exclusive_clips(&a).unwrap(), ["only-a.mp4"]);
        assert!(store.exclusive_clips(&b).unwrap().is_empty());
        assert!(store.exclusive_clips("f99").is_err());
    }

    #[test]
    fn listing_counts_only_clips_that_exist_and_forgets_nothing() {
        let dir = TempDir::new("folders-dangling");
        let store = store(&dir);
        let a = make(&store, "A");
        store
            .add_to_folder(&a, &names(&["here.mp4", "gone.mp4"]))
            .unwrap();
        let before = fs::read_to_string(dir.0.join("library.json")).unwrap();

        let info = &store.list(&[clip("here.mp4", 40)])[0];
        assert_eq!((info.clip_count, info.size_bytes), (1, 40));
        // An empty listing (say, the drive is unplugged) must not empty the index.
        assert_eq!(store.list(&[])[0].clip_count, 0);
        assert_eq!(
            fs::read_to_string(dir.0.join("library.json")).unwrap(),
            before
        );
        // ...and when the file comes back, so does the membership.
        assert_eq!(store.list(&[clip("gone.mp4", 1)])[0].clip_count, 1);
    }

    #[test]
    fn deleting_a_folder_removes_only_the_folder() {
        let dir = TempDir::new("folders-delete");
        let store = store(&dir);
        let a = make(&store, "A");
        let b = make(&store, "B");
        store.add_to_folder(&a, &names(&["x.mp4"])).unwrap();
        store.add_to_folder(&b, &names(&["x.mp4"])).unwrap();
        store.delete(&a).unwrap();

        let mut clips = vec![clip("x.mp4", 1)];
        store.annotate(&mut clips);
        assert_eq!(clips[0].folders, [b]);
        assert!(store.delete(&a).unwrap_err().contains("ya no existe"));
    }

    #[test]
    fn the_index_survives_a_restart() {
        let dir = TempDir::new("folders-reload");
        {
            let store = store(&dir);
            let a = make(&store, "Persistente");
            store.add_to_folder(&a, &names(&["x.mp4"])).unwrap();
            assert!(!dir.0.join("library.json.tmp").exists());
        }
        let again = store(&dir);
        let list = again.list(&[clip("x.mp4", 9)]);
        assert_eq!(list.len(), 1);
        assert_eq!(
            (list[0].name.as_str(), list[0].clip_count),
            ("Persistente", 1)
        );
        // The id counter came back too.
        assert_eq!(make(&again, "Siguiente"), "f2");
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_and_the_library_starts_empty() {
        let dir = TempDir::new("folders-corrupt");
        fs::write(dir.0.join("library.json"), "{ not json").unwrap();
        let store = store(&dir);
        assert!(store.list(&[]).is_empty());
        assert_eq!(
            fs::read_to_string(dir.0.join("library.json.bak")).unwrap(),
            "{ not json"
        );
        make(&store, "Nueva");
        assert_eq!(store.list(&[]).len(), 1);
    }

    #[test]
    fn a_hand_edited_file_is_tidied_on_load() {
        let dir = TempDir::new("folders-hand-edited");
        fs::write(
            dir.0.join("library.json"),
            r#"{"nextId":1,"folders":[
                {"id":"f7","name":"  Bien  ","color":"BAD COLOR","clips":["a.mp4","a.mp4","","b.mp4"]},
                {"id":"f7","name":"Repetida"},
                {"id":"","name":"Sin id"},
                {"id":"f2","name":"   ","icon":"x/y"}
            ]}"#,
        )
        .unwrap();
        let store = store(&dir);
        let clips = [clip("a.mp4", 1), clip("b.mp4", 1)];
        let list = store.list(&clips);
        let shown: Vec<_> = list.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(shown, ["Bien", "Carpeta f2"]);
        assert_eq!(list[0].color, DEFAULT_COLOR);
        assert_eq!(list[0].clip_count, 2, "duplicate and empty names dropped");
        assert_eq!(list[1].icon, DEFAULT_ICON);
        // New ids start above the highest one in the file.
        assert_eq!(make(&store, "Nueva"), "f8");
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let dir = TempDir::new("folders-failed-write");
        // The index "file" is a directory, so the rename must fail.
        fs::create_dir_all(dir.0.join("library.json")).unwrap();
        let store = store(&dir);
        assert!(store.create(&draft("No se guarda")).is_err());
        assert!(store.list(&[]).is_empty());
        // The id wasn't spent either.
        fs::remove_dir_all(dir.0.join("library.json")).unwrap();
        assert_eq!(make(&store, "Ahora sí"), "f1");
    }

    #[test]
    fn a_change_that_changes_nothing_does_not_write() {
        let dir = TempDir::new("folders-noop");
        let store = store(&dir);
        store.forget_clips(&names(&["x.mp4"])).unwrap();
        store.rename_clip("a.mp4", "b.mp4").unwrap();
        store.set_clip_folders("x.mp4", &[]).unwrap();
        assert!(!dir.0.join("library.json").exists());
    }

    // ---- automatic filing by game ----

    fn in_folder(store: &FolderStore, id: &str, clip_name: &str) -> bool {
        let mut clips = [clip(clip_name, 1)];
        store.annotate(&mut clips);
        clips[0].folders.iter().any(|f| f == id)
    }

    #[test]
    fn a_game_without_a_folder_gets_one_with_the_clip_in_it() {
        let dir = TempDir::new("folders-auto-new");
        let store = store(&dir);
        store.file_under_game("Valorant", "a.mp4").unwrap();

        let folders = store.list(&[clip("a.mp4", 10)]);
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "Valorant");
        assert_eq!(folders[0].icon, AUTO_ICON);
        assert!(AUTO_COLORS.contains(&folders[0].color.as_str()));
        assert_eq!(folders[0].clip_count, 1);
        assert!(in_folder(&store, &folders[0].id, "a.mp4"));
    }

    #[test]
    fn the_next_clip_of_the_same_game_goes_to_the_same_folder() {
        let dir = TempDir::new("folders-auto-same");
        let store = store(&dir);
        store.file_under_game("Valorant", "a.mp4").unwrap();
        store.file_under_game("Valorant", "b.mp4").unwrap();
        store.file_under_game("Valorant", "b.mp4").unwrap(); // saving twice changes nothing

        let folders = store.list(&[clip("a.mp4", 1), clip("b.mp4", 1)]);
        assert_eq!(folders.len(), 1, "no second folder");
        assert_eq!(folders[0].clip_count, 2);
    }

    #[test]
    fn capitals_and_spaces_do_not_make_a_new_folder() {
        let dir = TempDir::new("folders-auto-case");
        let store = store(&dir);
        store.file_under_game("Counter-Strike 2", "a.mp4").unwrap();
        store
            .file_under_game("  counter-strike 2 ", "b.mp4")
            .unwrap();
        store.file_under_game("COUNTER-STRIKE 2", "c.mp4").unwrap();
        let folders = store.list(&[]);
        assert_eq!(folders.len(), 1);
        assert_eq!(
            folders[0].name, "Counter-Strike 2",
            "the first spelling stays"
        );
    }

    #[test]
    fn a_folder_the_user_made_is_used_as_it_is() {
        let dir = TempDir::new("folders-auto-user");
        let store = store(&dir);
        let mine = make(&store, "valorant"); // blue star, made by hand
        store.add_to_folder(&mine, &names(&["old.mp4"])).unwrap();

        store.file_under_game("Valorant", "new.mp4").unwrap();

        let folders = store.list(&[clip("old.mp4", 1), clip("new.mp4", 1)]);
        assert_eq!(folders.len(), 1);
        assert_eq!(
            (
                folders[0].name.as_str(),
                folders[0].color.as_str(),
                folders[0].icon.as_str()
            ),
            ("valorant", "blue", "star"),
            "name, color and icon are the user's"
        );
        assert_eq!(folders[0].clip_count, 2, "its own clips stay");
    }

    #[test]
    fn filing_adds_to_the_clips_other_folders_and_does_not_move_it() {
        let dir = TempDir::new("folders-auto-keeps");
        let store = store(&dir);
        let best = make(&store, "Lo mejor");
        store.add_to_folder(&best, &names(&["a.mp4"])).unwrap();
        store.file_under_game("Valorant", "a.mp4").unwrap();
        let mut clips = [clip("a.mp4", 1)];
        store.annotate(&mut clips);
        assert_eq!(
            clips[0].folders.len(),
            2,
            "a clip may be in several folders"
        );
        assert!(clips[0].folders.contains(&best));
    }

    #[test]
    fn each_game_keeps_its_own_color() {
        // Stable on every run and platform, whatever the capitals.
        assert_eq!(auto_color("Valorant"), auto_color("VALORANT"));
        assert_eq!(auto_color("Valorant"), auto_color("valorant"));
        // Pinned: a change of hash would recolor everybody's folders.
        assert_eq!(auto_color(""), "teal");
        assert_eq!(auto_color("Valorant"), "pink");
        assert_eq!(auto_color("Counter-Strike 2"), "green");
        // A spread: a handful of games don't all land on one color.
        let colors: HashSet<_> = [
            "Valorant",
            "Counter-Strike 2",
            "Minecraft",
            "Fortnite",
            "Apex Legends",
            "Rocket League",
            "League of Legends",
            "Dota 2",
        ]
        .into_iter()
        .map(auto_color)
        .collect();
        assert!(colors.len() >= 4, "{colors:?}");
        assert!(
            AUTO_COLORS.iter().all(|c| style_ok(c)),
            "every id is a valid style"
        );
        assert!(style_ok(AUTO_ICON));
    }

    #[test]
    fn an_unusable_game_name_changes_nothing() {
        let dir = TempDir::new("folders-auto-bad");
        let store = store(&dir);
        for bad in ["", "   ", "tab\there"] {
            assert!(store.file_under_game(bad, "a.mp4").is_err(), "{bad:?}");
        }
        assert!(store.list(&[]).is_empty());
        assert!(!dir.0.join("library.json").exists(), "nothing was written");
    }

    #[test]
    fn at_the_folder_limit_an_existing_game_still_works_and_a_new_one_is_refused() {
        let dir = TempDir::new("folders-auto-limit");
        let store = store(&dir);
        for n in 0..MAX_FOLDERS - 1 {
            make(&store, &format!("Carpeta {n}"));
        }
        store.file_under_game("Valorant", "a.mp4").unwrap(); // the last free slot
        store.file_under_game("Valorant", "b.mp4").unwrap(); // reuses it
        let err = store.file_under_game("Minecraft", "c.mp4").unwrap_err();
        assert!(err.contains("más de"), "{err}");
        assert_eq!(store.list(&[]).len(), MAX_FOLDERS);
    }

    #[test]
    fn automatic_folders_survive_a_restart() {
        let dir = TempDir::new("folders-auto-persist");
        store(&dir).file_under_game("Valorant", "a.mp4").unwrap();
        let again = store(&dir);
        let folders = again.list(&[clip("a.mp4", 1)]);
        assert_eq!(folders[0].name, "Valorant");
        assert_eq!(folders[0].clip_count, 1);
        // The id counter went on: the next folder is not f1 again.
        assert_eq!(make(&again, "Otra"), "f2");
    }

    #[test]
    fn a_folder_deleted_by_the_user_is_made_again_for_the_next_clip() {
        let dir = TempDir::new("folders-auto-recreate");
        let store = store(&dir);
        store.file_under_game("Valorant", "a.mp4").unwrap();
        let first = store.list(&[])[0].id.clone();
        store.delete(&first).unwrap();
        store.file_under_game("Valorant", "b.mp4").unwrap();
        let folders = store.list(&[clip("b.mp4", 1)]);
        assert_eq!(folders.len(), 1);
        assert_ne!(folders[0].id, first, "ids are never reused");
        assert_eq!(folders[0].clip_count, 1, "only the new clip");
    }
}
