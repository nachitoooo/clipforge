//! Persistent app settings: one JSON file in the app config folder.
//!
//! Every field has a default (`#[serde(default)]`), so a file written by an
//! older version keeps loading after new settings are added, and a missing or
//! corrupt file simply means "start from the defaults". The store is free of
//! Tauri types so it can be unit tested; validation lives in
//! [`Settings::validated`], the one gate every incoming change goes through.

use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use crate::persist;

pub const SETTINGS_VERSION: u32 = 1;
/// Microphone gain bounds, in percent (100 = unchanged, 200 = +6 dB).
pub const GAIN_MIN_PERCENT: u32 = 0;
pub const GAIN_MAX_PERCENT: u32 = 200;
pub const DEFAULT_MUTE_SHORTCUT: &str = "CommandOrControl+Shift+M";
pub const DEFAULT_SAVE_SHORTCUT: &str = "CommandOrControl+Shift+S";
/// How much a save shortcut keeps by default, and the bounds of any length: a
/// clip can't be longer than the replay buffer (`BufferConfig::default`).
pub const DEFAULT_CLIP_SECONDS: u64 = 30;
pub const MIN_CLIP_SECONDS: u64 = 5;
pub const MAX_CLIP_SECONDS: u64 = 120;
/// Save shortcuts besides the main one.
pub const MAX_EXTRA_SAVE_SHORTCUTS: usize = 4;

/// How the microphone ends up in a saved clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MicMode {
    /// Voice and game share one audio track: plays everywhere, can't be
    /// muted afterwards.
    #[default]
    Mixed,
    /// Voice on its own track: can be muted afterwards, but many players
    /// (Discord, browsers) only play the first track.
    Separate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MicrophoneSettings {
    /// Off by default: recording your voice is opt-in.
    pub enabled: bool,
    /// Device id as reported by `list_microphones`; `None` = system default.
    pub device_id: Option<String>,
    pub mode: MicMode,
    pub gain_percent: u32,
    /// Silenced without disabling (the hotkey toggles this).
    pub muted: bool,
    /// Global accelerator that toggles `muted`, e.g. `CommandOrControl+Shift+M`.
    pub mute_shortcut: String,
}

impl Default for MicrophoneSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            device_id: None,
            mode: MicMode::Mixed,
            gain_percent: 100,
            muted: false,
            mute_shortcut: DEFAULT_MUTE_SHORTCUT.to_string(),
        }
    }
}

/// A save shortcut with a length of its own, e.g. F9 for the last 15 seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClipShortcut {
    pub accelerator: String,
    pub seconds: u64,
}

impl Default for ClipShortcut {
    fn default() -> Self {
        Self {
            accelerator: String::new(),
            seconds: DEFAULT_CLIP_SECONDS,
        }
    }
}

/// Global shortcuts that aren't tied to one feature's section (the
/// microphone's mute shortcut lives with the microphone).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShortcutSettings {
    /// Saves the last `clip_seconds` as a clip, from any app or game. The tray
    /// entry and the window's button save the same length.
    pub save_clip: String,
    pub clip_seconds: u64,
    /// More save shortcuts, each with its own length.
    pub extra_save_clip: Vec<ClipShortcut>,
    /// Takes a screenshot. Empty = no shortcut (the default: nothing is
    /// registered that nobody asked for; the tray and the window have a button).
    pub screenshot: String,
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            save_clip: DEFAULT_SAVE_SHORTCUT.to_string(),
            clip_seconds: DEFAULT_CLIP_SECONDS,
            extra_save_clip: Vec::new(),
            screenshot: String::new(),
        }
    }
}

fn clamp_seconds(seconds: u64) -> u64 {
    seconds.clamp(MIN_CLIP_SECONDS, MAX_CLIP_SECONDS)
}

/// A length as people say it: `30 s`, `1 min`, `1 min 30 s`.
pub fn clip_length_label(seconds: u64) -> String {
    match (seconds / 60, seconds % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

/// How the library learns about a clip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LibrarySettings {
    /// Tag each saved clip with the application that was in front while it was
    /// recorded (its "game"). Only the application's name is kept, on this
    /// machine; on by default, switchable off.
    pub detect_game: bool,
    /// File each new clip in a folder named after its game (made on the spot if
    /// there is none). Off by default: it creates folders. It needs the game, so
    /// it does nothing while `detect_game` is off.
    pub auto_folders: bool,
}

impl Default for LibrarySettings {
    fn default() -> Self {
        Self {
            detect_game: true,
            auto_folders: false,
        }
    }
}

/// How much the app may take from the computer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PerformanceSettings {
    /// Keep one minute of video at half the bitrate instead of two minutes, so
    /// the replay buffer takes a quarter of the memory (see `memory.rs`). Off by
    /// default. Changing it restarts the video capture and empties the buffer.
    pub low_ram: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub shortcuts: ShortcutSettings,
    pub microphone: MicrophoneSettings,
    pub library: LibrarySettings,
    pub performance: PerformanceSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            shortcuts: ShortcutSettings::default(),
            microphone: MicrophoneSettings::default(),
            library: LibrarySettings::default(),
            performance: PerformanceSettings::default(),
        }
    }
}

/// Keys that may be a shortcut on their own. None of them types a character
/// or edits text, so claiming one system-wide costs nothing anyone writes
/// with. Every other key needs Ctrl, Alt or ⌘ — alone, a letter would stop
/// working in every other application (and in ClipForge's own text boxes).
fn works_alone(key: Code) -> bool {
    use Code::*;
    matches!(
        key,
        F1 | F2
            | F3
            | F4
            | F5
            | F6
            | F7
            | F8
            | F9
            | F10
            | F11
            | F12
            | F13
            | F14
            | F15
            | F16
            | F17
            | F18
            | F19
            | F20
            | F21
            | F22
            | F23
            | F24
            | Insert
            | Home
            | End
            | PageUp
            | PageDown
            | Pause
            | ScrollLock
            | PrintScreen
    )
}

/// An accelerator that parses and is safe to register globally: a key that
/// works alone, or any key together with Ctrl, Alt or ⌘ (Shift alone doesn't
/// count: Shift+S still types a capital S).
fn parse_shortcut(accelerator: &str) -> Result<Shortcut, String> {
    let text = accelerator.trim();
    let shortcut = Shortcut::from_str(text).map_err(|_| format!("atajo no válido: «{text}»"))?;
    let modified = shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER);
    if !modified && !works_alone(shortcut.key) {
        return Err(format!(
            "el atajo «{text}» es una tecla para escribir o moverse: sola dejaría de funcionar en las demás aplicaciones. Combinala con Ctrl, Alt o ⌘, o usá una tecla de función (F1–F24), Insert, Inicio, Fin, RePág, AvPág, Pausa o ImprPant"
        ));
    }
    Ok(shortcut)
}

impl Settings {
    /// Whether going from `old` to `new` needs the video capture restarted — which
    /// empties the replay buffer, so the UI warns before it happens. Everything
    /// else is applied live.
    pub fn needs_capture_restart(old: &Settings, new: &Settings) -> bool {
        old.performance.low_ram != new.performance.low_ram
    }

    /// Normalize and check a settings object coming from the UI (or a file).
    /// Every global shortcut must parse and no two features may share one.
    pub fn validated(mut self) -> Result<Self, String> {
        self.version = SETTINGS_VERSION;

        let save_text = self.shortcuts.save_clip.trim().to_string();
        let save = parse_shortcut(&save_text)?;
        let mute_text = self.microphone.mute_shortcut.trim().to_string();
        let mute = parse_shortcut(&mute_text)?;
        if save == mute {
            return Err(format!(
                "el atajo «{save_text}» no puede ser a la vez para guardar clips y para silenciar el micrófono"
            ));
        }
        self.shortcuts.save_clip = save_text;
        self.microphone.mute_shortcut = mute_text;

        self.shortcuts.clip_seconds = clamp_seconds(self.shortcuts.clip_seconds);
        if self.shortcuts.extra_save_clip.len() > MAX_EXTRA_SAVE_SHORTCUTS {
            return Err(format!(
                "demasiados atajos para guardar clips: además del principal caben {MAX_EXTRA_SAVE_SHORTCUTS}"
            ));
        }
        let mut taken = vec![save, mute];
        for extra in &mut self.shortcuts.extra_save_clip {
            let text = extra.accelerator.trim().to_string();
            let shortcut = parse_shortcut(&text)?;
            if taken.contains(&shortcut) {
                return Err(format!(
                    "el atajo «{text}» ya está en uso: cada atajo tiene que ser distinto"
                ));
            }
            taken.push(shortcut);
            extra.accelerator = text;
            extra.seconds = clamp_seconds(extra.seconds);
        }

        let screenshot_text = self.shortcuts.screenshot.trim().to_string();
        if !screenshot_text.is_empty() {
            let shortcut = parse_shortcut(&screenshot_text)?;
            if taken.contains(&shortcut) {
                return Err(format!(
                    "el atajo «{screenshot_text}» ya está en uso: cada atajo tiene que ser distinto"
                ));
            }
        }
        self.shortcuts.screenshot = screenshot_text;

        let mic = &mut self.microphone;

        mic.gain_percent = mic.gain_percent.clamp(GAIN_MIN_PERCENT, GAIN_MAX_PERCENT);
        mic.device_id = mic
            .device_id
            .take()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());

        Ok(self)
    }

    /// Settings read from a file somebody may have edited by hand: keep what is
    /// usable and put anything that would stop the app from starting (a
    /// shortcut that doesn't parse, two features on the same keys) back to its
    /// default. The result always passes [`Settings::validated`].
    fn repaired(mut self) -> Self {
        if parse_shortcut(&self.shortcuts.save_clip).is_err() {
            self.shortcuts.save_clip = DEFAULT_SAVE_SHORTCUT.to_string();
        }
        if parse_shortcut(&self.microphone.mute_shortcut).is_err() {
            self.microphone.mute_shortcut = DEFAULT_MUTE_SHORTCUT.to_string();
        }
        if parse_shortcut(&self.shortcuts.save_clip).ok()
            == parse_shortcut(&self.microphone.mute_shortcut).ok()
        {
            self.microphone.mute_shortcut = DEFAULT_MUTE_SHORTCUT.to_string();
            if parse_shortcut(&self.shortcuts.save_clip).ok()
                == parse_shortcut(&self.microphone.mute_shortcut).ok()
            {
                self.shortcuts.save_clip = DEFAULT_SAVE_SHORTCUT.to_string();
            }
        }

        // The extra save shortcuts: keep the usable ones that are not repeated.
        self.shortcuts.clip_seconds = clamp_seconds(self.shortcuts.clip_seconds);
        let mut taken: Vec<Shortcut> = [&self.shortcuts.save_clip, &self.microphone.mute_shortcut]
            .into_iter()
            .filter_map(|text| parse_shortcut(text).ok())
            .collect();
        self.shortcuts.extra_save_clip.retain_mut(|extra| {
            match parse_shortcut(&extra.accelerator) {
                Ok(shortcut) if !taken.contains(&shortcut) => {
                    taken.push(shortcut);
                    extra.seconds = clamp_seconds(extra.seconds);
                    true
                }
                _ => false,
            }
        });
        self.shortcuts
            .extra_save_clip
            .truncate(MAX_EXTRA_SAVE_SHORTCUTS);

        // The screenshot shortcut is optional: one that can't be used is dropped.
        let screenshot_ok = match parse_shortcut(&self.shortcuts.screenshot) {
            Ok(shortcut) => !taken.contains(&shortcut),
            Err(_) => false,
        };
        if !screenshot_ok {
            self.shortcuts.screenshot = String::new();
        }
        self
    }
}

/// Loads, holds and saves the settings.
pub struct SettingsStore {
    path: PathBuf,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// Never fails: a missing file means defaults; an unreadable one is moved
    /// aside to `settings.json.bak` (so nothing is lost) and defaults apply.
    pub fn load(path: PathBuf) -> Self {
        let settings = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Settings>(&text)
                .map(Settings::repaired)
                .unwrap_or_else(|e| {
                    eprintln!("clipforge: settings file unreadable ({e}); using defaults");
                    persist::set_aside(&path);
                    Settings::default()
                }),
            Err(_) => Settings::default(),
        };
        Self {
            path,
            current: Mutex::new(settings),
        }
    }

    pub fn get(&self) -> Settings {
        self.current.lock().expect("settings lock").clone()
    }

    /// Persist `settings` (already validated) and make them current. The
    /// in-memory copy only changes if the write succeeds.
    pub fn replace(&self, settings: Settings) -> Result<Settings, String> {
        let json = serde_json::to_string_pretty(&settings)
            .map_err(|e| format!("no se pudieron serializar los ajustes: {e}"))?;
        persist::write_atomic(&self.path, json.as_bytes())
            .map_err(|e| format!("no se pudieron guardar los ajustes: {e}"))?;
        *self.current.lock().expect("settings lock") = settings.clone();
        Ok(settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scratch directory removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "clipforge-settings-{label}-{}",
                crate::testutil::unique()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("settings.json")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn defaults_are_private_and_sensible() {
        let s = Settings::default();
        assert!(!s.microphone.enabled, "the microphone is opt-in");
        assert_eq!(s.microphone.mode, MicMode::Mixed);
        assert_eq!(s.microphone.gain_percent, 100);
        assert!(!s.microphone.muted);
        assert_eq!(s.microphone.device_id, None);
        assert!(
            s.library.detect_game,
            "game detection is on unless switched off"
        );
        assert!(
            !s.library.auto_folders,
            "making folders by itself is opt-in"
        );
    }

    #[test]
    fn a_file_from_before_the_library_section_gets_its_default() {
        let dir = TempDir::new("no-library-section");
        fs::write(dir.file(), r#"{"microphone":{"enabled":true}}"#).unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert!(s.microphone.enabled);
        assert!(s.library.detect_game);
    }

    #[test]
    fn automatic_folders_can_be_switched_on_and_stay_on() {
        let dir = TempDir::new("auto-folders-on");
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.library.auto_folders = true;
        store.replace(s.validated().unwrap()).unwrap();
        let reloaded = SettingsStore::load(dir.file()).get();
        assert!(reloaded.library.auto_folders);
        assert!(
            reloaded.library.detect_game,
            "the other switch is untouched"
        );
    }

    #[test]
    fn a_library_section_from_before_automatic_folders_keeps_detection_and_gets_it_off() {
        let dir = TempDir::new("auto-folders-old");
        fs::write(dir.file(), r#"{"library":{"detectGame":false}}"#).unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert!(!s.library.detect_game);
        assert!(!s.library.auto_folders);

        let json = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(json["library"]["autoFolders"], false);
    }

    #[test]
    fn game_detection_can_be_switched_off_and_stays_off() {
        let dir = TempDir::new("detect-off");
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.library.detect_game = false;
        store.replace(s.validated().unwrap()).unwrap();
        assert!(!SettingsStore::load(dir.file()).get().library.detect_game);
    }

    #[test]
    fn missing_file_loads_defaults() {
        let dir = TempDir::new("missing");
        let store = SettingsStore::load(dir.file());
        assert_eq!(store.get(), Settings::default());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = TempDir::new("roundtrip");
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.microphone.enabled = true;
        s.microphone.device_id = Some("coreaudio:USB".into());
        s.microphone.mode = MicMode::Separate;
        s.microphone.gain_percent = 150;
        s.microphone.muted = true;
        store.replace(s.clone()).unwrap();

        let reloaded = SettingsStore::load(dir.file());
        assert_eq!(reloaded.get(), s);
    }

    #[test]
    fn a_file_from_an_older_version_keeps_loading() {
        let dir = TempDir::new("older");
        // No `microphone` section at all, plus a field we don't know about.
        fs::write(dir.file(), r#"{"version":1,"somethingNew":true}"#).unwrap();
        assert_eq!(SettingsStore::load(dir.file()).get(), Settings::default());

        // A partial microphone section fills the rest with defaults.
        fs::write(dir.file(), r#"{"microphone":{"enabled":true}}"#).unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert!(s.microphone.enabled);
        assert_eq!(s.microphone.gain_percent, 100);
        assert_eq!(s.microphone.mute_shortcut, DEFAULT_MUTE_SHORTCUT);
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_not_deleted() {
        let dir = TempDir::new("corrupt");
        fs::write(dir.file(), "{ this is not json").unwrap();
        let store = SettingsStore::load(dir.file());
        assert_eq!(store.get(), Settings::default());
        assert!(!dir.file().exists());
        let backup = dir.0.join("settings.json.bak");
        assert_eq!(fs::read_to_string(backup).unwrap(), "{ this is not json");
    }

    #[test]
    fn saving_creates_the_folder_and_leaves_no_temp_file() {
        let dir = TempDir::new("atomic");
        let path = dir.0.join("nested").join("settings.json");
        let store = SettingsStore::load(path.clone());
        store.replace(Settings::default()).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn a_failed_write_keeps_the_previous_settings() {
        let dir = TempDir::new("failed-write");
        // The settings "file" is a directory, so the rename must fail.
        fs::create_dir_all(dir.file()).unwrap();
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.microphone.enabled = true;
        assert!(store.replace(s).is_err());
        assert!(!store.get().microphone.enabled);
    }

    #[test]
    fn validation_clamps_gain_and_cleans_the_device_id() {
        let mut s = Settings::default();
        s.microphone.gain_percent = 9_999;
        s.microphone.device_id = Some("   ".into());
        let v = s.validated().unwrap();
        assert_eq!(v.microphone.gain_percent, GAIN_MAX_PERCENT);
        assert_eq!(v.microphone.device_id, None);

        let mut s = Settings::default();
        s.microphone.device_id = Some("  coreaudio:USB  ".into());
        let v = s.validated().unwrap();
        assert_eq!(v.microphone.device_id.as_deref(), Some("coreaudio:USB"));
    }

    #[test]
    fn validation_rejects_bad_and_conflicting_shortcuts() {
        let mut s = Settings::default();
        s.microphone.mute_shortcut = "Banana".into();
        assert!(s.validated().unwrap_err().contains("no válido"));

        let mut s = Settings::default();
        s.microphone.mute_shortcut = "CommandOrControl+Shift+S".into();
        assert!(s.validated().unwrap_err().contains("no puede ser a la vez"));
    }

    #[test]
    fn the_default_shortcuts_are_valid_and_distinct() {
        let s = Settings::default();
        assert_eq!(s.shortcuts.save_clip, DEFAULT_SAVE_SHORTCUT);
        assert!(s.validated().is_ok());
    }

    #[test]
    fn the_save_shortcut_can_be_changed() {
        let mut s = Settings::default();
        s.shortcuts.save_clip = "  Alt+F9  ".into();
        let v = s.validated().unwrap();
        assert_eq!(v.shortcuts.save_clip, "Alt+F9", "trimmed");
    }

    #[test]
    fn a_bad_save_shortcut_is_refused() {
        for bad in ["", "Banana", "Shift+", "Ctrl+Ctrl+"] {
            let mut s = Settings::default();
            s.shortcuts.save_clip = bad.into();
            let err = s.validated().unwrap_err();
            assert!(err.contains("no válido"), "{bad:?}: {err}");
        }
    }

    fn with_save(accelerator: &str) -> Settings {
        let mut s = Settings::default();
        s.shortcuts.save_clip = accelerator.into();
        s
    }

    #[test]
    fn keys_that_do_not_type_anything_work_on_their_own() {
        for key in [
            "F1",
            "F9",
            "F12",
            "F13",
            "F24",
            "Insert",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Pause",
            "PrintScreen",
            "ScrollLock",
            "Shift+F9",
        ] {
            let v = with_save(key)
                .validated()
                .unwrap_or_else(|e| panic!("{key} should work alone: {e}"));
            assert_eq!(v.shortcuts.save_clip, key);
            // The mute shortcut follows the same rule.
            let mut s = Settings::default();
            s.microphone.mute_shortcut = key.into();
            assert!(s.validated().is_ok(), "{key} as the mute shortcut");
        }
    }

    #[test]
    fn keys_used_for_writing_need_a_real_modifier() {
        for key in [
            "S",
            "1",
            "Space",
            "Enter",
            "Tab",
            "Backspace",
            "Delete",
            "ArrowUp",
            "ArrowLeft",
            "Numpad0",
            "NumpadAdd",
            "NumpadEnter",
            "Escape",
            ".",
            "-",
            "Shift+S",
            "Shift+Space",
            "Shift+ArrowDown",
        ] {
            let err = with_save(key).validated().unwrap_err();
            assert!(err.contains("sola dejaría de funcionar"), "{key}: {err}");
            assert!(err.contains(key), "the message names the shortcut: {err}");
        }
    }

    #[test]
    fn the_same_keys_are_fine_with_ctrl_alt_or_command() {
        for key in [
            "Control+S",
            "Alt+Space",
            "Super+Enter",
            "CommandOrControl+Numpad0",
            "Control+Alt+ArrowUp",
            "Alt+Shift+S",
            "CommandOrControl+Backspace",
        ] {
            assert!(with_save(key).validated().is_ok(), "{key}");
        }
    }

    #[test]
    fn a_hand_edited_file_with_a_typing_key_alone_is_repaired() {
        let dir = TempDir::new("bare-letter");
        fs::write(dir.file(), r#"{"shortcuts":{"saveClip":"S"}}"#).unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.save_clip, DEFAULT_SAVE_SHORTCUT);

        // ...while a function key alone is kept.
        fs::write(dir.file(), r#"{"shortcuts":{"saveClip":"F9"}}"#).unwrap();
        assert_eq!(
            SettingsStore::load(dir.file()).get().shortcuts.save_clip,
            "F9"
        );
    }

    #[test]
    fn the_two_shortcuts_cannot_be_the_same_however_they_are_spelled() {
        let mut s = Settings::default();
        s.shortcuts.save_clip = "CmdOrCtrl+Shift+M".into(); // the mute default
        assert!(s.validated().unwrap_err().contains("no puede ser a la vez"));

        let mut s = Settings::default();
        s.microphone.mute_shortcut = "Shift+CommandOrControl+S".into(); // the save default
        assert!(s.validated().unwrap_err().contains("no puede ser a la vez"));

        // Swapping the two is fine.
        let mut s = Settings::default();
        s.shortcuts.save_clip = DEFAULT_MUTE_SHORTCUT.into();
        s.microphone.mute_shortcut = DEFAULT_SAVE_SHORTCUT.into();
        assert!(s.validated().is_ok());
    }

    #[test]
    fn a_file_without_shortcuts_gets_the_default_save_shortcut() {
        let dir = TempDir::new("no-shortcuts");
        fs::write(dir.file(), r#"{"microphone":{"enabled":true}}"#).unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.save_clip, DEFAULT_SAVE_SHORTCUT);
    }

    #[test]
    fn the_save_shortcut_survives_a_restart() {
        let dir = TempDir::new("shortcut-roundtrip");
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.shortcuts.save_clip = "Alt+F9".into();
        store.replace(s).unwrap();
        assert_eq!(
            SettingsStore::load(dir.file()).get().shortcuts.save_clip,
            "Alt+F9"
        );
    }

    #[test]
    fn a_hand_edited_file_cannot_stop_the_app_from_starting() {
        let dir = TempDir::new("repair");

        // A shortcut that doesn't parse goes back to its default.
        fs::write(
            dir.file(),
            r#"{"shortcuts":{"saveClip":"nonsense"},"microphone":{"muteShortcut":""}}"#,
        )
        .unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.save_clip, DEFAULT_SAVE_SHORTCUT);
        assert_eq!(s.microphone.mute_shortcut, DEFAULT_MUTE_SHORTCUT);

        // Two features on the same keys: the microphone gives way...
        fs::write(
            dir.file(),
            r#"{"shortcuts":{"saveClip":"Alt+F9"},"microphone":{"muteShortcut":"Alt+F9"}}"#,
        )
        .unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.save_clip, "Alt+F9");
        assert_eq!(s.microphone.mute_shortcut, DEFAULT_MUTE_SHORTCUT);

        // ...unless that is where it was sent, then everything is the default.
        fs::write(
            dir.file(),
            r#"{"shortcuts":{"saveClip":"CommandOrControl+Shift+M"},"microphone":{"muteShortcut":"CommandOrControl+Shift+M"}}"#,
        )
        .unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.save_clip, DEFAULT_SAVE_SHORTCUT);
        assert_eq!(s.microphone.mute_shortcut, DEFAULT_MUTE_SHORTCUT);

        // Whatever it repaired passes validation.
        assert!(s.validated().is_ok());
    }

    #[test]
    fn json_uses_camel_case_for_the_ui() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert!(json["microphone"]["gainPercent"].is_number());
        assert!(json["microphone"]["deviceId"].is_null());
        assert_eq!(json["microphone"]["mode"], "mixed");
        assert_eq!(json["shortcuts"]["saveClip"], "CommandOrControl+Shift+S");
        assert_eq!(json["library"]["detectGame"], true);
    }

    #[test]
    fn every_accelerator_the_ui_can_record_is_accepted() {
        // The shapes produced by `captureAccelerator` in app/src/lib/hotkey.ts.
        for accelerator in [
            "CommandOrControl+Shift+M",
            "Control+K",
            "Super+K",
            "CommandOrControl+Alt+Shift+F5",
            "Alt+7",
            "Alt+Space",
            "Alt+ArrowUp",
            "CommandOrControl+Alt+ArrowDown",
            "Control+Alt+ArrowLeft",
            "Alt+ArrowRight",
            "Alt+F24",
            "CommandOrControl+Z",
            "Alt+F1",
            "F9",
            "Shift+F9",
            "Insert",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Pause",
            "PrintScreen",
            "ScrollLock",
            "Control+Enter",
            "Control+Tab",
            "Control+Backspace",
            "Control+Delete",
            "Control+Backquote",
            "Control+Minus",
            "Control+Equal",
            "Control+BracketLeft",
            "Control+BracketRight",
            "Control+Backslash",
            "Control+Semicolon",
            "Control+Comma",
            "Control+Period",
            "Control+Slash",
            "Control+Numpad0",
            "Control+Numpad9",
            "Control+NumpadAdd",
            "Control+NumpadSubtract",
            "Control+NumpadMultiply",
            "Control+NumpadDivide",
            "Control+NumpadDecimal",
            "Control+NumpadEnter",
            "Control+NumpadEqual",
        ] {
            let mut s = Settings::default();
            s.microphone.mute_shortcut = accelerator.to_string();
            s.shortcuts.save_clip = "Control+Alt+Shift+F12".to_string();
            assert!(
                s.validated().is_ok(),
                "{accelerator} should be accepted as the mute shortcut"
            );
            let mut s = Settings::default();
            s.shortcuts.save_clip = accelerator.to_string();
            s.microphone.mute_shortcut = "Control+Alt+Shift+F12".to_string();
            assert!(
                s.validated().is_ok(),
                "{accelerator} should be accepted as the save shortcut"
            );
        }
    }

    // ---- save shortcuts with their own length ----

    fn extra(accelerator: &str, seconds: u64) -> ClipShortcut {
        ClipShortcut {
            accelerator: accelerator.into(),
            seconds,
        }
    }

    #[test]
    fn a_clip_is_30_seconds_by_default_and_no_longer_than_the_buffer() {
        let s = Settings::default();
        assert_eq!(s.shortcuts.clip_seconds, 30);
        assert!(s.shortcuts.extra_save_clip.is_empty());
        assert_eq!(
            MAX_CLIP_SECONDS,
            clipforge_core::buffer::BufferConfig::default()
                .max_duration
                .as_secs(),
            "a longer clip could never be filled"
        );
    }

    #[test]
    fn lengths_are_clamped_to_what_the_buffer_can_hold() {
        let mut s = Settings::default();
        s.shortcuts.clip_seconds = 0;
        s.shortcuts.extra_save_clip = vec![extra("F9", 9_999), extra("F10", 1)];
        let v = s.validated().unwrap();
        assert_eq!(v.shortcuts.clip_seconds, MIN_CLIP_SECONDS);
        assert_eq!(v.shortcuts.extra_save_clip[0].seconds, MAX_CLIP_SECONDS);
        assert_eq!(v.shortcuts.extra_save_clip[1].seconds, MIN_CLIP_SECONDS);

        let mut s = Settings::default();
        s.shortcuts.clip_seconds = 45;
        s.shortcuts.extra_save_clip = vec![extra("  F9 ", 15)];
        let v = s.validated().unwrap();
        assert_eq!(v.shortcuts.clip_seconds, 45);
        assert_eq!(
            v.shortcuts.extra_save_clip,
            vec![extra("F9", 15)],
            "trimmed"
        );
    }

    #[test]
    fn every_save_shortcut_must_be_different() {
        for (taken, label) in [
            ("CmdOrCtrl+Shift+S", "the main save shortcut"),
            ("CommandOrControl+Shift+M", "the mute shortcut"),
            (
                "shift+commandorcontrol+m",
                "the mute shortcut spelled differently",
            ),
        ] {
            let mut s = Settings::default();
            s.shortcuts.extra_save_clip = vec![extra(taken, 15)];
            let err = s.validated().unwrap_err();
            assert!(err.contains("ya está en uso"), "{label}: {err}");
        }

        let mut s = Settings::default();
        s.shortcuts.extra_save_clip = vec![extra("F9", 15), extra("f9", 60)];
        assert!(s.validated().unwrap_err().contains("ya está en uso"));
    }

    #[test]
    fn an_extra_shortcut_follows_the_same_rules_as_any_other() {
        for bad in ["", "Banana", "S", "Space"] {
            let mut s = Settings::default();
            s.shortcuts.extra_save_clip = vec![extra(bad, 15)];
            assert!(s.validated().is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn there_is_a_limit_to_the_extra_shortcuts() {
        let keys = ["F5", "F6", "F7", "F8", "F9"];
        let mut s = Settings::default();
        s.shortcuts.extra_save_clip = keys[..MAX_EXTRA_SAVE_SHORTCUTS]
            .iter()
            .map(|k| extra(k, 15))
            .collect();
        assert!(s.clone().validated().is_ok());

        s.shortcuts
            .extra_save_clip
            .push(extra(keys[MAX_EXTRA_SAVE_SHORTCUTS], 15));
        assert!(s.validated().unwrap_err().contains("demasiados"));
    }

    #[test]
    fn extra_shortcuts_survive_a_restart() {
        let dir = TempDir::new("extra-shortcuts");
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.shortcuts.clip_seconds = 60;
        s.shortcuts.extra_save_clip = vec![extra("F9", 15), extra("Alt+F9", 120)];
        store.replace(s.clone().validated().unwrap()).unwrap();
        assert_eq!(SettingsStore::load(dir.file()).get(), s);
    }

    #[test]
    fn a_file_from_before_clip_lengths_keeps_its_shortcut_and_gets_30_seconds() {
        let dir = TempDir::new("before-lengths");
        fs::write(dir.file(), r#"{"shortcuts":{"saveClip":"F9"}}"#).unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.save_clip, "F9");
        assert_eq!(s.shortcuts.clip_seconds, 30);
        assert!(s.shortcuts.extra_save_clip.is_empty());
    }

    #[test]
    fn a_hand_edited_file_keeps_only_the_usable_extra_shortcuts() {
        let dir = TempDir::new("extra-repair");
        fs::write(
            dir.file(),
            r#"{"shortcuts":{"saveClip":"F9","clipSeconds":99999,"extraSaveClip":[
                {"accelerator":"F10","seconds":15},
                {"accelerator":"nonsense","seconds":15},
                {"accelerator":"f9","seconds":15},
                {"accelerator":"CommandOrControl+Shift+M","seconds":15},
                {"accelerator":"S","seconds":15},
                {"accelerator":"F10","seconds":60},
                {"accelerator":"F11","seconds":0},
                {"accelerator":"F12"},
                {"accelerator":"F13","seconds":30},
                {"accelerator":"F14","seconds":30}
            ]}}"#,
        )
        .unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.clip_seconds, MAX_CLIP_SECONDS);
        assert_eq!(
            s.shortcuts.extra_save_clip,
            vec![
                extra("F10", 15),
                extra("F11", MIN_CLIP_SECONDS),
                extra("F12", DEFAULT_CLIP_SECONDS),
                extra("F13", 30),
            ],
            "unusable and repeated ones are dropped, the rest capped at four"
        );
        assert!(s.validated().is_ok(), "what it repaired passes validation");
    }

    #[test]
    fn repairing_the_main_shortcut_does_not_leave_an_extra_on_the_same_keys() {
        let dir = TempDir::new("extra-vs-default");
        // "nonsense" falls back to the default; an extra already on those keys goes.
        fs::write(
            dir.file(),
            r#"{"shortcuts":{"saveClip":"nonsense","extraSaveClip":[{"accelerator":"CommandOrControl+Shift+S","seconds":15}]}}"#,
        )
        .unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.save_clip, DEFAULT_SAVE_SHORTCUT);
        assert!(s.shortcuts.extra_save_clip.is_empty());
        assert!(s.validated().is_ok());
    }

    #[test]
    fn lengths_read_the_way_people_say_them() {
        for (seconds, label) in [
            (5, "5 s"),
            (30, "30 s"),
            (59, "59 s"),
            (60, "1 min"),
            (90, "1 min 30 s"),
            (120, "2 min"),
        ] {
            assert_eq!(clip_length_label(seconds), label);
        }
    }

    #[test]
    fn the_ui_sees_the_lengths_in_camel_case() {
        let mut s = Settings::default();
        s.shortcuts.extra_save_clip = vec![extra("F9", 15)];
        let json = serde_json::to_value(s).unwrap();
        assert_eq!(json["shortcuts"]["clipSeconds"], 30);
        assert_eq!(json["shortcuts"]["extraSaveClip"][0]["accelerator"], "F9");
        assert_eq!(json["shortcuts"]["extraSaveClip"][0]["seconds"], 15);
    }

    // ---- the screenshot shortcut ----

    #[test]
    fn there_is_no_screenshot_shortcut_until_one_is_chosen() {
        let s = Settings::default();
        assert_eq!(s.shortcuts.screenshot, "");
        assert!(s.clone().validated().is_ok());
        let json = serde_json::to_value(s).unwrap();
        assert_eq!(json["shortcuts"]["screenshot"], "");
    }

    #[test]
    fn a_screenshot_shortcut_follows_the_same_rules_as_the_others() {
        let mut s = Settings::default();
        s.shortcuts.screenshot = "  F8 ".into();
        assert_eq!(s.validated().unwrap().shortcuts.screenshot, "F8", "trimmed");

        for bad in ["Banana", "S", "Space"] {
            let mut s = Settings::default();
            s.shortcuts.screenshot = bad.into();
            assert!(s.validated().is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn the_screenshot_shortcut_cannot_share_keys_with_any_other() {
        for (other, label) in [
            ("CmdOrCtrl+Shift+S", "the main save shortcut"),
            ("CommandOrControl+Shift+M", "the mute shortcut"),
            ("f9", "an extra save shortcut"),
        ] {
            let mut s = Settings::default();
            s.shortcuts.extra_save_clip = vec![extra("F9", 15)];
            s.shortcuts.screenshot = other.into();
            let err = s.validated().unwrap_err();
            assert!(err.contains("ya está en uso"), "{label}: {err}");
        }
    }

    #[test]
    fn the_screenshot_shortcut_survives_a_restart_and_can_be_cleared() {
        let dir = TempDir::new("screenshot-shortcut");
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.shortcuts.screenshot = "F8".into();
        store.replace(s.clone().validated().unwrap()).unwrap();
        assert_eq!(
            SettingsStore::load(dir.file()).get().shortcuts.screenshot,
            "F8"
        );

        s.shortcuts.screenshot = String::new();
        store.replace(s.validated().unwrap()).unwrap();
        assert_eq!(
            SettingsStore::load(dir.file()).get().shortcuts.screenshot,
            ""
        );
    }

    #[test]
    fn a_file_from_before_screenshots_has_no_screenshot_shortcut() {
        let dir = TempDir::new("before-screenshots");
        fs::write(
            dir.file(),
            r#"{"shortcuts":{"saveClip":"F9","clipSeconds":60}}"#,
        )
        .unwrap();
        let s = SettingsStore::load(dir.file()).get();
        assert_eq!(s.shortcuts.screenshot, "");
        assert_eq!(s.shortcuts.save_clip, "F9");
    }

    #[test]
    fn a_hand_edited_screenshot_shortcut_that_cannot_work_is_dropped() {
        for (text, label) in [
            ("nonsense", "does not parse"),
            ("S", "a letter alone"),
            ("CommandOrControl+Shift+S", "the save default"),
            ("F9", "an extra one"),
        ] {
            let dir = TempDir::new("screenshot-repair");
            fs::write(
                dir.file(),
                format!(
                    r#"{{"shortcuts":{{"screenshot":"{text}","extraSaveClip":[{{"accelerator":"F9","seconds":15}}]}}}}"#
                ),
            )
            .unwrap();
            let s = SettingsStore::load(dir.file()).get();
            assert_eq!(s.shortcuts.screenshot, "", "{label}");
            assert_eq!(
                s.shortcuts.extra_save_clip.len(),
                1,
                "{label}: the rest is kept"
            );
            assert!(s.validated().is_ok());
        }

        // A good one stays.
        let dir = TempDir::new("screenshot-repair-ok");
        fs::write(dir.file(), r#"{"shortcuts":{"screenshot":"F8"}}"#).unwrap();
        assert_eq!(
            SettingsStore::load(dir.file()).get().shortcuts.screenshot,
            "F8"
        );
    }

    // ---- low RAM mode ----

    #[test]
    fn low_ram_is_off_until_chosen_and_old_files_do_not_have_it() {
        assert!(!Settings::default().performance.low_ram);
        let dir = TempDir::new("low-ram-old");
        fs::write(dir.file(), r#"{"microphone":{"enabled":true}}"#).unwrap();
        assert!(!SettingsStore::load(dir.file()).get().performance.low_ram);
    }

    #[test]
    fn low_ram_survives_a_restart_and_is_camel_case() {
        let dir = TempDir::new("low-ram-on");
        let store = SettingsStore::load(dir.file());
        let mut s = Settings::default();
        s.performance.low_ram = true;
        store.replace(s.validated().unwrap()).unwrap();
        assert!(SettingsStore::load(dir.file()).get().performance.low_ram);
        let json = serde_json::to_value(SettingsStore::load(dir.file()).get()).unwrap();
        assert_eq!(json["performance"]["lowRam"], true);
    }

    #[test]
    fn only_a_change_of_memory_mode_restarts_the_capture() {
        let off = Settings::default();
        let mut on = off.clone();
        on.performance.low_ram = true;
        assert!(Settings::needs_capture_restart(&off, &on));
        assert!(Settings::needs_capture_restart(&on, &off));
        assert!(!Settings::needs_capture_restart(&on, &on));

        // Nothing else asks for one.
        let mut other = off.clone();
        other.microphone.enabled = true;
        other.microphone.gain_percent = 150;
        other.shortcuts.clip_seconds = 60;
        other.shortcuts.screenshot = "F8".into();
        other.library.auto_folders = true;
        assert!(!Settings::needs_capture_restart(&off, &other));
    }
}
