//! A real screenshot of the real screen. Ignored: it needs a logged-in macOS 14+
//! desktop and the Screen Recording permission for the terminal or IDE.
//! `cargo test -p clipforge-core --test macos_screenshot -- --ignored --nocapture`
#![cfg(target_os = "macos")]

use std::fs;

use clipforge_core::screenshot::{capture_png, png_dimensions, unique_path};

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("clipforge-shot-{label}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "needs a macOS 14+ desktop with Screen Recording permission"]
fn the_screen_is_saved_as_a_png_of_the_size_it_reports() {
    let dir = scratch("real");
    let path = unique_path(&dir, "20261012-000000");

    let shot = capture_png(&path).expect("a screenshot (is Screen Recording allowed?)");
    println!("{}x{} → {}", shot.width, shot.height, path.display());

    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        png_dimensions(&bytes),
        Some((shot.width, shot.height)),
        "the file says what the call said"
    );
    assert!(shot.width >= 640 && shot.height >= 400, "{shot:?}");
    assert!(
        bytes.len() > 20_000,
        "a real screen is not a few bytes ({})",
        bytes.len()
    );
    assert!(
        !fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")),
        "no temporary file is left"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs a macOS 14+ desktop with Screen Recording permission"]
fn two_pictures_in_a_row_are_two_files() {
    let dir = scratch("two");
    let first = unique_path(&dir, "20261012-000000");
    capture_png(&first).unwrap();
    let second = unique_path(&dir, "20261012-000000");
    assert_ne!(first, second);
    capture_png(&second).unwrap();
    assert!(first.is_file() && second.is_file());
    fs::remove_dir_all(&dir).ok();
}
