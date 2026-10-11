//! The foreground-application lookup against the real macOS session. Ignored:
//! it needs a logged-in desktop (no permission prompt, but no headless CI).
//! `cargo test -p clipforge-core --test macos_activity -- --ignored --nocapture`
#![cfg(target_os = "macos")]

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clipforge_core::activity::{system_source, ActivityLog, ActivityTracker};

#[test]
#[ignore = "needs a logged-in macOS desktop session"]
fn the_application_in_front_is_named() {
    let mut source = system_source().expect("macOS has a foreground source");
    let app = source.foreground().expect("some application is in front");
    println!("in front: {} ({})", app.name, app.id);
    assert!(!app.name.trim().is_empty());
    assert!(!app.id.trim().is_empty());
}

#[test]
#[ignore = "needs a logged-in macOS desktop session"]
fn the_tracker_fills_a_log_from_the_real_source() {
    let log = Arc::new(Mutex::new(ActivityLog::new()));
    let tracker = ActivityTracker::start(
        system_source().expect("macOS has a foreground source"),
        Arc::clone(&log),
        Arc::new(AtomicBool::new(true)),
        Duration::from_millis(100),
    );
    thread::sleep(Duration::from_millis(600));
    drop(tracker);

    let log = log.lock().unwrap();
    assert!(!log.is_empty(), "the tracker never recorded anything");
    let who = log.dominant(
        Instant::now(),
        Duration::from_secs(1),
        Duration::ZERO,
        |_| false,
    );
    println!("dominant over the last second: {who:?}");
    assert!(who.is_some());
}
