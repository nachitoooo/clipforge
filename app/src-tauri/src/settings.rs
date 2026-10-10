//! Persistent app settings: one JSON file in the app config folder.
//!
//! Every field has a default (`#[serde(default)]`), so a file written by an
//! older version keeps loading after new settings are added, and a missing or
//! corrupt file simply means "start from the defaults". The store is free of
//! Tauri types so it can be unit tested; validation lives in
//! [`Settings::validated`], the one gate every incoming change goes through.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri_plugin_global_shortcut::Shortcut;

pub const SETTINGS_VERSION: u32 = 1;
/// Microphone gain bounds, in percent (100 = unchanged, 200 = +6 dB).
pub const GAIN_MIN_PERCENT: u32 = 0;
pub const GAIN_MAX_PERCENT: u32 = 200;
pub const DEFAULT_MUTE_SHORTCUT: &str = "CommandOrControl+Shift+M";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub microphone: MicrophoneSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            microphone: MicrophoneSettings::default(),
        }
    }
}

impl Settings {
    /// Normalize and check a settings object coming from the UI (or a file).
    /// `reserved` lists shortcuts other features own; the mute shortcut must
    /// not collide with them.
    pub fn validated(mut self, reserved: &[Shortcut]) -> Result<Self, String> {
        self.version = SETTINGS_VERSION;
        let mic = &mut self.microphone;

        mic.gain_percent = mic.gain_percent.clamp(GAIN_MIN_PERCENT, GAIN_MAX_PERCENT);
        mic.device_id = mic
            .device_id
            .take()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());

        let accelerator = mic.mute_shortcut.trim();
        let shortcut = Shortcut::from_str(accelerator)
            .map_err(|_| format!("atajo no válido: «{accelerator}»"))?;
        if reserved.contains(&shortcut) {
            return Err(format!(
                "el atajo «{accelerator}» ya lo usa otra función de ClipForge"
            ));
        }
        mic.mute_shortcut = accelerator.to_string();
        Ok(self)
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
            Ok(text) => serde_json::from_str::<Settings>(&text).unwrap_or_else(|e| {
                eprintln!("clipforge: settings file unreadable ({e}); using defaults");
                let backup = path.with_extension("json.bak");
                fs::rename(&path, backup).ok();
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
        write_atomic(&self.path, json.as_bytes())
            .map_err(|e| format!("no se pudieron guardar los ajustes: {e}"))?;
        *self.current.lock().expect("settings lock") = settings.clone();
        Ok(settings)
    }
}

/// Write to a sibling temp file, then rename over the target, so a crash
/// mid-write can never leave a half-written settings file.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Scratch directory removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "clipforge-settings-{label}-{}-{nanos}",
                std::process::id()
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

    fn save_clip() -> Shortcut {
        Shortcut::from_str("CommandOrControl+Shift+S").unwrap()
    }

    #[test]
    fn defaults_are_private_and_sensible() {
        let s = Settings::default();
        assert!(!s.microphone.enabled, "the microphone is opt-in");
        assert_eq!(s.microphone.mode, MicMode::Mixed);
        assert_eq!(s.microphone.gain_percent, 100);
        assert!(!s.microphone.muted);
        assert_eq!(s.microphone.device_id, None);
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
        let v = s.validated(&[save_clip()]).unwrap();
        assert_eq!(v.microphone.gain_percent, GAIN_MAX_PERCENT);
        assert_eq!(v.microphone.device_id, None);

        let mut s = Settings::default();
        s.microphone.device_id = Some("  coreaudio:USB  ".into());
        let v = s.validated(&[save_clip()]).unwrap();
        assert_eq!(v.microphone.device_id.as_deref(), Some("coreaudio:USB"));
    }

    #[test]
    fn validation_rejects_bad_and_conflicting_shortcuts() {
        let mut s = Settings::default();
        s.microphone.mute_shortcut = "Banana".into();
        assert!(s
            .validated(&[save_clip()])
            .unwrap_err()
            .contains("no válido"));

        let mut s = Settings::default();
        s.microphone.mute_shortcut = "CommandOrControl+Shift+S".into();
        assert!(s
            .validated(&[save_clip()])
            .unwrap_err()
            .contains("ya lo usa"));
    }

    #[test]
    fn the_default_shortcut_is_valid_and_free() {
        assert!(Settings::default().validated(&[save_clip()]).is_ok());
    }

    #[test]
    fn json_uses_camel_case_for_the_ui() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        assert!(json["microphone"]["gainPercent"].is_number());
        assert!(json["microphone"]["deviceId"].is_null());
        assert_eq!(json["microphone"]["mode"], "mixed");
    }
}
