//! Smaller Windows checks that need a real session: which application is in
//! front, and putting a file on the clipboard. `#[ignore]`d —
//! `cargo test -p clipforge-core --test windows_misc -- --ignored --nocapture`.
#![cfg(target_os = "windows")]

use clipforge_core::activity::system_source;
use clipforge_core::clipboard::copy_file;
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::Ole::CF_HDROP;

#[test]
#[ignore = "needs a Windows desktop session"]
fn the_application_in_front_is_named() {
    let mut source = system_source().expect("Windows has a foreground source");
    // On a CI desktop the foreground window may belong to a process we may not
    // inspect; naming something is what matters when there is something.
    match source.foreground() {
        Some(app) => {
            eprintln!("in front: {} ({})", app.name, app.id);
            assert!(!app.name.trim().is_empty() && !app.id.trim().is_empty());
        }
        None => eprintln!("no application in front (nothing readable on this desktop)"),
    }
}

#[test]
#[ignore = "needs a Windows desktop session"]
fn a_file_goes_to_the_clipboard_as_a_drop_list() {
    let path = std::env::temp_dir().join(format!("clipforge-clipboard-{}.mp4", std::process::id()));
    std::fs::write(&path, b"not really a clip").unwrap();
    copy_file(&path).expect("the clipboard accepts the file");
    // SAFETY: plain query.
    assert!(
        unsafe { IsClipboardFormatAvailable(u32::from(CF_HDROP.0)) }.is_ok(),
        "the clipboard holds a file list"
    );
    std::fs::remove_file(&path).ok();
}
