//! What the app knows about each clip beyond the file itself. Today that is
//! the **game** — the application that held the screen while it was recorded —
//! which the library shows and filters by.
//!
//! Like the folder index (`folders.rs`) this is a sidecar keyed by clip file
//! name, in `clipmeta.json` next to `settings.json`, and it follows the same
//! rules: reads never write or prune (a name whose file is missing is inert,
//! not forgotten), every change is all or nothing, and the app's own actions
//! that rename, delete or copy a clip keep it in step.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::library::ClipInfo;
use crate::persist::JsonStore;

pub const META_VERSION: u32 = 1;
pub const MAX_GAME_CHARS: usize = 60;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ClipMeta {
    game: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Index {
    version: u32,
    /// Clip file name → what is known about it. A clip that has nothing to
    /// say has no entry.
    clips: BTreeMap<String, ClipMeta>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            version: META_VERSION,
            clips: BTreeMap::new(),
        }
    }
}

/// A game name the user typed: trimmed, 1–60 characters, no control characters.
pub fn validate_game(raw: &str) -> Result<String, String> {
    let game = raw.trim();
    if game.is_empty() {
        return Err("el nombre del juego no puede estar vacío".into());
    }
    if game.chars().count() > MAX_GAME_CHARS {
        return Err(format!(
            "el nombre del juego no puede superar {MAX_GAME_CHARS} caracteres"
        ));
    }
    if game.chars().any(char::is_control) {
        return Err("el nombre del juego contiene caracteres no permitidos".into());
    }
    Ok(game.to_string())
}

/// A game name the OS reported: never refused (it isn't the user's typing),
/// only made presentable. `None` when nothing is left.
pub fn clean_game(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    let game: String = cleaned.trim().chars().take(MAX_GAME_CHARS).collect();
    let game = game.trim().to_string();
    (!game.is_empty()).then_some(game)
}

fn same_game(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl Index {
    /// Tidy a file somebody edited by hand: presentable names, no empty entries.
    fn repaired(mut self) -> Self {
        for meta in self.clips.values_mut() {
            meta.game = meta.game.take().and_then(|g| clean_game(&g));
        }
        self.clips
            .retain(|name, meta| !name.is_empty() && *meta != ClipMeta::default());
        self.version = META_VERSION;
        self
    }

    /// The spelling to store for `game`: the one already in use by a clip
    /// outside `except`, if any (so "valorant" and "Valorant" are one game).
    fn canonical(&self, game: &str, except: impl Fn(&str) -> bool) -> String {
        self.clips
            .values()
            .filter_map(|meta| meta.game.as_deref())
            .find(|existing| !except(existing) && same_game(existing, game))
            .unwrap_or(game)
            .to_string()
    }

    fn set(&mut self, name: &str, game: Option<String>) {
        match game {
            Some(game) => {
                self.clips
                    .insert(name.to_string(), ClipMeta { game: Some(game) });
            }
            None => {
                self.clips.remove(name);
            }
        }
    }
}

/// Loads, holds and saves the per-clip facts.
pub struct ClipMetaStore {
    store: JsonStore<Index>,
}

impl ClipMetaStore {
    /// Never fails: a missing file means nothing is known; an unreadable one is
    /// moved aside to `clipmeta.json.bak` and starts empty.
    pub fn load(path: PathBuf) -> Self {
        Self {
            store: JsonStore::load(path, "los datos de los clips", Index::repaired),
        }
    }

    /// Set (or clear, with `None`) the game of a clip the OS named. A name
    /// that is already in use elsewhere keeps that spelling.
    pub fn set_detected_game(&self, name: &str, detected: Option<&str>) -> Result<(), String> {
        let game = detected.and_then(clean_game);
        self.store.mutate(|index| {
            let game = game.map(|g| index.canonical(&g, |_| false));
            index.set(name, game);
            Ok(())
        })
    }

    /// Set (or clear, with `None`) the game of a clip, as typed by the user.
    pub fn set_game(&self, name: &str, game: Option<&str>) -> Result<(), String> {
        let game = game.map(validate_game).transpose()?;
        self.store.mutate(|index| {
            let game = game.map(|g| index.canonical(&g, |_| false));
            index.set(name, game);
            Ok(())
        })
    }

    /// The game of one clip.
    pub fn game_of(&self, name: &str) -> Option<String> {
        self.store
            .read(|index| index.clips.get(name).and_then(|m| m.game.clone()))
    }

    /// The user corrected the game of a clip and wants every clip of the same
    /// game corrected too: `new` replaces the clip's current game everywhere.
    /// A clip with no game yet only changes itself. Returns how many clips
    /// changed.
    pub fn set_game_everywhere(&self, name: &str, new: &str) -> Result<usize, String> {
        match self.game_of(name) {
            Some(current) => self.rename_game(&current, new),
            None => self.set_game(name, Some(new)).map(|()| 1),
        }
    }

    /// Call every clip of `from` by `to` instead; returns how many changed.
    /// Renaming to a game that already exists merges the two.
    pub fn rename_game(&self, from: &str, to: &str) -> Result<usize, String> {
        let to = validate_game(to)?;
        self.store.mutate(|index| {
            let target = index.canonical(&to, |existing| same_game(existing, from));
            let mut changed = 0;
            for meta in index.clips.values_mut() {
                if meta.game.as_deref().is_some_and(|g| same_game(g, from))
                    && meta.game.as_deref() != Some(target.as_str())
                {
                    meta.game = Some(target.clone());
                    changed += 1;
                }
            }
            Ok(changed)
        })
    }

    /// The clips are gone: forget them.
    pub fn forget_clips(&self, names: &[String]) -> Result<(), String> {
        self.store.mutate(|index| {
            index.clips.retain(|name, _| !names.contains(name));
            Ok(())
        })
    }

    /// A clip was renamed: what is known about it follows.
    pub fn rename_clip(&self, old: &str, new: &str) -> Result<(), String> {
        self.store.mutate(|index| {
            if let Some(meta) = index.clips.remove(old) {
                index.clips.insert(new.to_string(), meta);
            }
            Ok(())
        })
    }

    /// `new` is a copy of `source` (a trimmed clip): same facts.
    pub fn inherit(&self, source: &str, new: &str) -> Result<(), String> {
        self.store.mutate(|index| {
            if let Some(meta) = index.clips.get(source).cloned() {
                index.clips.insert(new.to_string(), meta);
            }
            Ok(())
        })
    }

    /// Fill in the game of each clip.
    pub fn annotate(&self, clips: &mut [ClipInfo]) {
        self.store.read(|index| {
            for clip in clips {
                clip.game = index.clips.get(&clip.name).and_then(|m| m.game.clone());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::fs;

    fn store(dir: &TempDir) -> ClipMetaStore {
        ClipMetaStore::load(dir.0.join("clipmeta.json"))
    }

    fn clip(name: &str) -> ClipInfo {
        ClipInfo {
            name: name.into(),
            kind: crate::library::ClipKind::Clip,
            path: format!("/clips/{name}"),
            size_bytes: 1,
            modified_ms: 0,
            folders: Vec::new(),
            game: None,
        }
    }

    fn games(store: &ClipMetaStore, names: &[&str]) -> Vec<Option<String>> {
        let mut clips: Vec<ClipInfo> = names.iter().map(|n| clip(n)).collect();
        store.annotate(&mut clips);
        clips.into_iter().map(|c| c.game).collect()
    }

    fn some(game: &str) -> Option<String> {
        Some(game.to_string())
    }

    #[test]
    fn a_clip_remembers_its_game() {
        let dir = TempDir::new("meta-record");
        let store = store(&dir);
        store.set_detected_game("a.mp4", Some("Valorant")).unwrap();
        assert_eq!(games(&store, &["a.mp4", "b.mp4"]), [some("Valorant"), None]);
    }

    #[test]
    fn no_detected_game_clears_a_stale_entry() {
        let dir = TempDir::new("meta-stale");
        let store = store(&dir);
        store.set_detected_game("a.mp4", Some("Old")).unwrap();
        store.set_detected_game("a.mp4", None).unwrap();
        assert_eq!(games(&store, &["a.mp4"]), [None]);
    }

    #[test]
    fn a_detected_name_is_made_presentable_never_refused() {
        let dir = TempDir::new("meta-clean");
        let store = store(&dir);
        store
            .set_detected_game("a.mp4", Some("  Valo\trant\n "))
            .unwrap();
        store
            .set_detected_game("b.mp4", Some(&"x".repeat(200)))
            .unwrap();
        store.set_detected_game("c.mp4", Some("   ")).unwrap();
        let found = games(&store, &["a.mp4", "b.mp4", "c.mp4"]);
        assert_eq!(found[0], some("Valorant"));
        assert_eq!(found[1].as_ref().unwrap().chars().count(), MAX_GAME_CHARS);
        assert_eq!(found[2], None);
    }

    #[test]
    fn a_typed_name_is_checked() {
        let dir = TempDir::new("meta-typed");
        let store = store(&dir);
        for bad in ["", "   ", "tab\there", &"x".repeat(MAX_GAME_CHARS + 1)] {
            assert!(store.set_game("a.mp4", Some(bad)).is_err(), "{bad:?}");
        }
        store.set_game("a.mp4", Some("  Ñandú Racing  ")).unwrap();
        assert_eq!(games(&store, &["a.mp4"]), [some("Ñandú Racing")]);
        store.set_game("a.mp4", None).unwrap();
        assert_eq!(games(&store, &["a.mp4"]), [None]);
    }

    #[test]
    fn the_same_game_in_other_capitals_keeps_the_first_spelling() {
        let dir = TempDir::new("meta-canonical");
        let store = store(&dir);
        store.set_detected_game("a.mp4", Some("VALORANT")).unwrap();
        store.set_detected_game("b.mp4", Some("valorant")).unwrap();
        store.set_game("c.mp4", Some("Valorant")).unwrap();
        assert_eq!(
            games(&store, &["a.mp4", "b.mp4", "c.mp4"]),
            [some("VALORANT"), some("VALORANT"), some("VALORANT")]
        );
    }

    #[test]
    fn renaming_a_game_changes_every_clip_of_it_and_counts_them() {
        let dir = TempDir::new("meta-rename-game");
        let store = store(&dir);
        for name in ["a.mp4", "b.mp4"] {
            store
                .set_detected_game(name, Some("VALORANT-Win64-Shipping"))
                .unwrap();
        }
        store.set_detected_game("c.mp4", Some("Other")).unwrap();

        let changed = store
            .rename_game("valorant-win64-shipping", "Valorant")
            .unwrap();
        assert_eq!(changed, 2);
        assert_eq!(
            games(&store, &["a.mp4", "b.mp4", "c.mp4"]),
            [some("Valorant"), some("Valorant"), some("Other")]
        );
        assert_eq!(store.rename_game("Nothing here", "X").unwrap(), 0);
        assert!(store.rename_game("Valorant", "  ").is_err());
    }

    #[test]
    fn correcting_one_clip_can_correct_all_of_its_game() {
        let dir = TempDir::new("meta-everywhere");
        let store = store(&dir);
        store
            .set_detected_game("a.mp4", Some("game-win64"))
            .unwrap();
        store
            .set_detected_game("b.mp4", Some("game-win64"))
            .unwrap();
        store.set_detected_game("c.mp4", Some("Other")).unwrap();

        assert_eq!(store.set_game_everywhere("a.mp4", "Game").unwrap(), 2);
        assert_eq!(
            games(&store, &["a.mp4", "b.mp4", "c.mp4"]),
            [some("Game"), some("Game"), some("Other")]
        );
        // A clip without a game only changes itself.
        assert_eq!(store.set_game_everywhere("d.mp4", "Game").unwrap(), 1);
        assert_eq!(store.game_of("d.mp4"), some("Game"));
        assert_eq!(store.game_of("nope.mp4"), None);
        assert!(store.set_game_everywhere("a.mp4", "  ").is_err());
    }

    #[test]
    fn renaming_a_game_can_only_change_its_capitals() {
        let dir = TempDir::new("meta-rename-capitals");
        let store = store(&dir);
        store.set_detected_game("a.mp4", Some("valorant")).unwrap();
        assert_eq!(store.rename_game("valorant", "Valorant").unwrap(), 1);
        assert_eq!(games(&store, &["a.mp4"]), [some("Valorant")]);
    }

    #[test]
    fn renaming_a_game_onto_an_existing_one_merges_them() {
        let dir = TempDir::new("meta-merge");
        let store = store(&dir);
        store
            .set_detected_game("a.mp4", Some("Counter-Strike 2"))
            .unwrap();
        store.set_detected_game("b.mp4", Some("cs2")).unwrap();
        assert_eq!(store.rename_game("cs2", "COUNTER-STRIKE 2").unwrap(), 1);
        assert_eq!(
            games(&store, &["a.mp4", "b.mp4"]),
            [some("Counter-Strike 2"), some("Counter-Strike 2")]
        );
    }

    #[test]
    fn what_is_known_follows_a_renamed_clip() {
        let dir = TempDir::new("meta-rename-clip");
        let store = store(&dir);
        store.set_detected_game("old.mp4", Some("Game")).unwrap();
        store.rename_clip("old.mp4", "new.mp4").unwrap();
        assert_eq!(games(&store, &["old.mp4", "new.mp4"]), [None, some("Game")]);
    }

    #[test]
    fn a_deleted_clip_is_forgotten() {
        let dir = TempDir::new("meta-forget");
        let store = store(&dir);
        store.set_detected_game("a.mp4", Some("Game")).unwrap();
        store.set_detected_game("b.mp4", Some("Game")).unwrap();
        store.forget_clips(&["a.mp4".to_string()]).unwrap();
        assert_eq!(games(&store, &["a.mp4", "b.mp4"]), [None, some("Game")]);
    }

    #[test]
    fn a_trimmed_copy_has_the_game_of_the_original() {
        let dir = TempDir::new("meta-inherit");
        let store = store(&dir);
        store.set_detected_game("clip.mp4", Some("Game")).unwrap();
        store.inherit("clip.mp4", "clip-recorte.mp4").unwrap();
        store.inherit("nothing.mp4", "nothing-recorte.mp4").unwrap();
        assert_eq!(
            games(
                &store,
                &["clip.mp4", "clip-recorte.mp4", "nothing-recorte.mp4"]
            ),
            [some("Game"), some("Game"), None]
        );
    }

    #[test]
    fn it_survives_a_restart_and_leaves_no_temp_file() {
        let dir = TempDir::new("meta-reload");
        store(&dir)
            .set_detected_game("a.mp4", Some("Game"))
            .unwrap();
        assert!(!dir.0.join("clipmeta.json.tmp").exists());
        assert_eq!(games(&store(&dir), &["a.mp4"]), [some("Game")]);
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_and_nothing_is_known() {
        let dir = TempDir::new("meta-corrupt");
        fs::write(dir.0.join("clipmeta.json"), "{ not json").unwrap();
        let store = store(&dir);
        assert_eq!(games(&store, &["a.mp4"]), [None]);
        assert_eq!(
            fs::read_to_string(dir.0.join("clipmeta.json.bak")).unwrap(),
            "{ not json"
        );
        store.set_detected_game("a.mp4", Some("Game")).unwrap();
        assert_eq!(games(&store, &["a.mp4"]), [some("Game")]);
    }

    #[test]
    fn a_hand_edited_file_is_tidied_on_load() {
        let dir = TempDir::new("meta-hand-edited");
        fs::write(
            dir.0.join("clipmeta.json"),
            r#"{"version":9,"clips":{
                "a.mp4":{"game":"  Spaced  "},
                "b.mp4":{"game":"   "},
                "c.mp4":{},
                "":{"game":"No name"}
            }}"#,
        )
        .unwrap();
        let store = store(&dir);
        assert_eq!(
            games(&store, &["a.mp4", "b.mp4", "c.mp4", ""]),
            [some("Spaced"), None, None, None]
        );
    }

    #[test]
    fn reading_never_writes_and_unknown_names_are_inert() {
        let dir = TempDir::new("meta-reads");
        let store = store(&dir);
        store.set_detected_game("gone.mp4", Some("Game")).unwrap();
        let before = fs::read_to_string(dir.0.join("clipmeta.json")).unwrap();
        // Listing a folder where the clip isn't: nothing shown, nothing pruned.
        assert_eq!(games(&store, &["other.mp4"]), [None]);
        assert_eq!(
            fs::read_to_string(dir.0.join("clipmeta.json")).unwrap(),
            before
        );
        // It comes back with the file.
        assert_eq!(games(&store, &["gone.mp4"]), [some("Game")]);
    }

    #[test]
    fn a_change_that_changes_nothing_does_not_write() {
        let dir = TempDir::new("meta-noop");
        let store = store(&dir);
        store.set_detected_game("a.mp4", None).unwrap();
        store.forget_clips(&["a.mp4".to_string()]).unwrap();
        store.rename_clip("a.mp4", "b.mp4").unwrap();
        store.inherit("a.mp4", "b.mp4").unwrap();
        assert_eq!(store.rename_game("Game", "Other").unwrap(), 0);
        assert!(!dir.0.join("clipmeta.json").exists());
    }

    #[test]
    fn a_failed_write_changes_nothing() {
        let dir = TempDir::new("meta-failed-write");
        // The file "is" a directory, so the rename must fail.
        fs::create_dir_all(dir.0.join("clipmeta.json")).unwrap();
        let store = store(&dir);
        assert!(store.set_detected_game("a.mp4", Some("Game")).is_err());
        assert_eq!(games(&store, &["a.mp4"]), [None]);
    }

    #[test]
    fn the_json_uses_camel_case_and_names_each_clip() {
        let dir = TempDir::new("meta-json");
        store(&dir)
            .set_detected_game("a.mp4", Some("Game"))
            .unwrap();
        let text = fs::read_to_string(dir.0.join("clipmeta.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["clips"]["a.mp4"]["game"], "Game");
    }
}
