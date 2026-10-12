//! The file's URL on `NSPasteboard` — what the Finder's "Copy" puts there, so
//! Discord, Slack, Mail, Messages and the Finder itself paste it as a file.
//! Needs no permission.

use std::path::Path;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSString, NSURL};

/// Copy `path` to the general (system) clipboard.
pub fn copy_file(path: &Path) -> Result<(), String> {
    write_file(&NSPasteboard::generalPasteboard(), path)
}

/// Replace whatever `pasteboard` holds with the file at `path`. Split from
/// [`copy_file`] so the test can use a private pasteboard instead of the
/// user's clipboard.
pub fn write_file(pasteboard: &NSPasteboard, path: &Path) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| "la ruta del archivo no es texto válido".to_string())?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(text));
    let item: Retained<ProtocolObject<dyn NSPasteboardWriting>> =
        ProtocolObject::from_retained(url);
    let objects = NSArray::from_retained_slice(&[item]);
    pasteboard.clearContents();
    if pasteboard.writeObjects(&objects) {
        Ok(())
    } else {
        Err("macOS no dejó copiar el archivo al portapapeles".to_string())
    }
}
