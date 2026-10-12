//! Copying a file to the pasteboard, checked against AppKit itself. Ignored: it
//! needs a logged-in macOS desktop session (the pasteboard server). It writes to
//! a private pasteboard, never to the user's clipboard.
//! `cargo test -p clipforge-core --test macos_clipboard -- --ignored --nocapture`
#![cfg(target_os = "macos")]

use std::fs;
use std::path::PathBuf;

use clipforge_core::clipboard::macos::write_file;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeFileURL};
use objc2_foundation::NSURL;

fn scratch_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("clipforge-pb-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, b"not really a clip").unwrap();
    path
}

/// The file the pasteboard's first item points at, read back the way an
/// application pasting it would.
fn pasted_path(pasteboard: &NSPasteboard) -> Option<String> {
    // SAFETY: the type constant is a plain NSString provided by AppKit.
    let url_text = pasteboard.stringForType(unsafe { NSPasteboardTypeFileURL })?;
    NSURL::URLWithString(&url_text)?
        .path()
        .map(|path| path.to_string())
}

#[test]
#[ignore = "needs a logged-in macOS desktop session"]
fn the_file_can_be_pasted_back_as_a_file() {
    let pasteboard = NSPasteboard::pasteboardWithUniqueName();
    let path = scratch_file("clip-de-prueba.mp4");

    write_file(&pasteboard, &path).expect("the pasteboard accepts the file");

    let pasted = pasted_path(&pasteboard).expect("a file URL is on the pasteboard");
    // /var and /private/var are the same place; compare what the file system says.
    assert_eq!(
        fs::canonicalize(&pasted).unwrap(),
        fs::canonicalize(&path).unwrap()
    );
    let types = pasteboard.types().expect("types");
    // SAFETY: as above.
    let file_url = unsafe { NSPasteboardTypeFileURL };
    assert!(
        (0..types.count()).any(|i| types.objectAtIndex(i).isEqualToString(file_url)),
        "offered as a file, not only as text"
    );
}

#[test]
#[ignore = "needs a logged-in macOS desktop session"]
fn spaces_and_accents_in_the_name_come_back_intact() {
    let pasteboard = NSPasteboard::pasteboardWithUniqueName();
    let path = scratch_file("Mi clip ñandú #1 (final).mp4");

    write_file(&pasteboard, &path).unwrap();

    let pasted = pasted_path(&pasteboard).expect("a file URL is on the pasteboard");
    // AppKit may hand the name back in decomposed Unicode (n + a combining
    // tilde): what matters is that it opens the very same file.
    assert_eq!(
        fs::canonicalize(&pasted).unwrap(),
        fs::canonicalize(&path).unwrap(),
        "{pasted}"
    );
}

#[test]
#[ignore = "needs a logged-in macOS desktop session"]
fn copying_again_replaces_the_previous_file() {
    let pasteboard = NSPasteboard::pasteboardWithUniqueName();
    let first = scratch_file("uno.mp4");
    let second = scratch_file("dos.mp4");

    write_file(&pasteboard, &first).unwrap();
    write_file(&pasteboard, &second).unwrap();

    let pasted = pasted_path(&pasteboard).unwrap();
    assert!(pasted.ends_with("dos.mp4"), "{pasted}");
}
