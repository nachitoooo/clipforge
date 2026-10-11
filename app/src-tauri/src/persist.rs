//! Small helpers shared by the files the app keeps in its config folder
//! (`settings.json`, `library.json`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Write to a sibling temp file, then rename over the target, so a crash
/// mid-write can never leave a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Move an unreadable file aside to `<name>.json.bak` so nothing the user had
/// is lost, and the caller can carry on with defaults.
pub fn set_aside(path: &Path) {
    fs::rename(path, path.with_extension("json.bak")).ok();
}

/// One JSON file held in memory, with the rules every index of ours follows:
/// * loading never fails — a missing file means the default, an unreadable one
///   is moved aside to `.json.bak` and the default applies;
/// * a change is made on a copy and kept only if the file was written, so
///   memory and disk never disagree, and a change that changes nothing doesn't
///   touch the disk;
/// * reading never writes.
pub struct JsonStore<T> {
    path: PathBuf,
    /// What the messages call this file: "la biblioteca".
    what: &'static str,
    value: Mutex<T>,
}

impl<T: Serialize + DeserializeOwned + Clone + PartialEq + Default> JsonStore<T> {
    /// `repair` tidies a file somebody edited by hand.
    pub fn load(path: PathBuf, what: &'static str, repair: impl FnOnce(T) -> T) -> Self {
        let value = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<T>(&text)
                .map(repair)
                .unwrap_or_else(|e| {
                    eprintln!("clipforge: {what} unreadable ({e}); starting empty");
                    set_aside(&path);
                    T::default()
                }),
            Err(_) => T::default(),
        };
        Self {
            path,
            what,
            value: Mutex::new(value),
        }
    }

    /// Apply `change` to a copy and keep it only if it is saved.
    pub fn mutate<R>(&self, change: impl FnOnce(&mut T) -> Result<R, String>) -> Result<R, String> {
        let mut current = self.value.lock().expect("json store lock");
        let mut next = current.clone();
        let out = change(&mut next)?;
        if next != *current {
            let json = serde_json::to_string_pretty(&next)
                .map_err(|e| format!("no se pudo serializar {}: {e}", self.what))?;
            write_atomic(&self.path, json.as_bytes())
                .map_err(|e| format!("no se pudo guardar {}: {e}", self.what))?;
            *current = next;
        }
        Ok(out)
    }

    pub fn read<R>(&self, view: impl FnOnce(&T) -> R) -> R {
        view(&self.value.lock().expect("json store lock"))
    }
}
