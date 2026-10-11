//! Small helpers shared by the files the app keeps in its config folder
//! (`settings.json`, `library.json`).

use std::fs;
use std::path::Path;

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
