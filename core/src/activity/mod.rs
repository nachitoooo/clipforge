//! What the user is doing on the machine while the buffer runs: which
//! application is in front. A saved clip is tagged with the one that held the
//! screen for most of the clip, so the library can show and filter by game.
//!
//! Only the application's *name* is read, never its window contents or title,
//! and it never leaves the machine. The pieces:
//! * [`ForegroundSource`] — the per-OS question "who is in front now?"
//!   ([`system_source`]); tests use a fake.
//! * [`ActivityLog`] — pure bookkeeping: samples over time and
//!   [`ActivityLog::dominant`], the application that held the screen longest.
//! * [`ActivityTracker`] — a thread that samples a source into a log.

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How far back the log remembers.
pub const LOG_SPAN: Duration = Duration::from_secs(15 * 60);
/// How often the tracker looks.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

/// An application, as the OS names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppInfo {
    /// Stable identity: the bundle id on macOS (the lowercased name when the
    /// program has none, as a development build), the lowercased exe name on
    /// Windows.
    pub id: String,
    /// What to show the user.
    pub name: String,
}

/// "Which application is in front right now?"
pub trait ForegroundSource: Send {
    /// `None` when nothing can be told (no window in front, access denied).
    fn foreground(&mut self) -> Option<AppInfo>;
}

/// The real source of this OS, if there is one.
pub fn system_source() -> Option<Box<dyn ForegroundSource>> {
    #[cfg(target_os = "macos")]
    {
        Some(Box::new(macos::MacForeground))
    }
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(windows::WindowsForeground))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Applications that are never "the game": ClipForge itself (saving from its
/// own button or the tray would otherwise tag every clip with it) and the
/// desktop shells that are in front while switching between windows.
pub fn default_ignore(app: &AppInfo) -> bool {
    const IGNORED: &[&str] = &[
        "com.clipforge.app",
        "clipforge",
        "clipforge-app",
        "clipforge.exe",
        "clipforge-app.exe",
        "com.apple.finder",
        "com.apple.dock",
        "com.apple.loginwindow",
        "explorer.exe",
        "searchhost.exe",
        "shellexperiencehost.exe",
        "startmenuexperiencehost.exe",
        "applicationframehost.exe",
        "lockapp.exe",
    ];
    IGNORED.iter().any(|id| app.id.eq_ignore_ascii_case(id))
}

/// Who was in front over time. Samples are `(when, who)`: each one holds until
/// the next. Consecutive repeats are not stored.
#[derive(Debug, Default)]
pub struct ActivityLog {
    samples: VecDeque<(Instant, Option<AppInfo>)>,
}

impl ActivityLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn record(&mut self, at: Instant, app: Option<AppInfo>) {
        if self.samples.back().is_some_and(|(_, last)| *last == app) {
            return;
        }
        self.samples.push_back((at, app));
        // Forget what is older than the span, but keep the sample that was
        // already in force at the cutoff: it still covers the time after it.
        if let Some(cutoff) = at.checked_sub(LOG_SPAN) {
            while self.samples.len() >= 2 && self.samples[1].0 <= cutoff {
                self.samples.pop_front();
            }
        }
    }

    /// The application (not ignored) that was in front for the longest part of
    /// the last `window` before `now`. It must have held the screen for at
    /// least `min`, so a quick glance at something else doesn't name the clip.
    /// A tie goes to whoever was in front last.
    pub fn dominant(
        &self,
        now: Instant,
        window: Duration,
        min: Duration,
        ignore: impl Fn(&AppInfo) -> bool,
    ) -> Option<AppInfo> {
        let from = now.checked_sub(window);
        // (who, time held, when it last held the screen)
        let mut held: Vec<(&AppInfo, Duration, Instant)> = Vec::new();
        for (i, (start, app)) in self.samples.iter().enumerate() {
            let Some(app) = app else { continue };
            if ignore(app) {
                continue;
            }
            let end = self
                .samples
                .get(i + 1)
                .map_or(now, |(next, _)| *next)
                .min(now);
            let start = from.map_or(*start, |from| (*start).max(from));
            if end <= start {
                continue;
            }
            let time = end - start;
            match held.iter_mut().find(|(who, _, _)| *who == app) {
                Some(entry) => {
                    entry.1 += time;
                    entry.2 = end;
                }
                None => held.push((app, time, end)),
            }
        }
        held.into_iter()
            .filter(|(_, time, _)| *time >= min)
            .max_by_key(|(_, time, last)| (*time, *last))
            .map(|(app, _, _)| app.clone())
    }
}

/// A thread that looks at a [`ForegroundSource`] every `interval` and writes
/// what it sees into a shared [`ActivityLog`]. While `enabled` is false it
/// looks at nothing and forgets what it had. Dropping it stops the thread.
pub struct ActivityTracker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ActivityTracker {
    pub fn start(
        mut source: Box<dyn ForegroundSource>,
        log: Arc<Mutex<ActivityLog>>,
        enabled: Arc<AtomicBool>,
        interval: Duration,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("clipforge-activity".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        if enabled.load(Ordering::Relaxed) {
                            let app = source.foreground();
                            log.lock()
                                .expect("activity log")
                                .record(Instant::now(), app);
                        } else {
                            log.lock().expect("activity log").clear();
                        }
                        sleep_unless_stopped(&stop, interval);
                    }
                })
                .ok()
        };
        Self { stop, thread }
    }
}

/// Sleep for `total`, waking early if `stop` is set (so stopping is prompt).
fn sleep_unless_stopped(stop: &AtomicBool, total: Duration) {
    const SLICE: Duration = Duration::from_millis(25);
    let deadline = Instant::now() + total;
    while !stop.load(Ordering::Relaxed) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        thread::sleep(left.min(SLICE));
    }
}

impl Drop for ActivityTracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> AppInfo {
        AppInfo {
            id: id.into(),
            name: id.to_uppercase(),
        }
    }

    const NOTHING: fn(&AppInfo) -> bool = |_| false;
    const ZERO: Duration = Duration::ZERO;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// A log whose samples are given as (seconds after `t0`, who).
    fn log_of(t0: Instant, samples: &[(u64, Option<&str>)]) -> ActivityLog {
        let mut log = ActivityLog::new();
        for (at, who) in samples {
            log.record(t0 + secs(*at), who.map(app));
        }
        log
    }

    #[test]
    fn the_one_in_front_longest_wins() {
        let t0 = Instant::now();
        let log = log_of(
            t0,
            &[(0, Some("game")), (20, Some("chat")), (25, Some("game"))],
        );
        // 0-20 and 25-30 → game 25 s; chat 5 s.
        let who = log.dominant(t0 + secs(30), secs(30), ZERO, NOTHING);
        assert_eq!(who, Some(app("game")));
    }

    #[test]
    fn only_the_window_counts() {
        let t0 = Instant::now();
        let log = log_of(t0, &[(0, Some("old-game")), (60, Some("new-game"))]);
        // The last 30 s are all new-game.
        assert_eq!(
            log.dominant(t0 + secs(90), secs(30), ZERO, NOTHING),
            Some(app("new-game"))
        );
        // The last 100 s: old-game 60 s vs new-game 30 s.
        assert_eq!(
            log.dominant(t0 + secs(90), secs(100), ZERO, NOTHING),
            Some(app("old-game"))
        );
    }

    #[test]
    fn a_sample_in_force_before_the_window_still_counts_inside_it() {
        let t0 = Instant::now();
        let log = log_of(t0, &[(0, Some("game"))]);
        assert_eq!(
            log.dominant(t0 + secs(500), secs(30), ZERO, NOTHING),
            Some(app("game"))
        );
    }

    #[test]
    fn ignored_apps_never_win_and_their_time_is_not_counted() {
        let t0 = Instant::now();
        let log = log_of(
            t0,
            &[
                (0, Some("game")),
                (10, Some("com.clipforge.app")),
                (30, Some("game")),
            ],
        );
        // ClipForge held the screen for 20 s, the game for 10 + 10.
        let who = log.dominant(t0 + secs(40), secs(40), ZERO, default_ignore);
        assert_eq!(who, Some(app("game")));

        let only_ours = log_of(t0, &[(0, Some("com.clipforge.app"))]);
        assert_eq!(
            only_ours.dominant(t0 + secs(30), secs(30), ZERO, default_ignore),
            None
        );
    }

    #[test]
    fn nothing_in_front_means_no_answer() {
        let t0 = Instant::now();
        let log = log_of(t0, &[(0, None), (10, Some("game")), (20, None)]);
        // Only 10 s of "game"; with a 15 s minimum that is not enough.
        assert_eq!(
            log.dominant(t0 + secs(30), secs(30), secs(15), NOTHING),
            None
        );
        assert_eq!(
            log.dominant(t0 + secs(30), secs(30), secs(5), NOTHING),
            Some(app("game"))
        );
        assert_eq!(
            ActivityLog::new().dominant(t0, secs(30), ZERO, NOTHING),
            None
        );
    }

    #[test]
    fn a_glance_at_something_else_does_not_name_the_clip() {
        let t0 = Instant::now();
        let log = log_of(t0, &[(0, Some("clipforge-ish")), (28, Some("browser"))]);
        // The browser held the screen 2 s of 30: below a fifth of the window.
        let who = log.dominant(t0 + secs(30), secs(30), secs(6), NOTHING);
        assert_eq!(who, Some(app("clipforge-ish")));
    }

    #[test]
    fn a_tie_goes_to_the_one_that_was_in_front_last() {
        let t0 = Instant::now();
        let log = log_of(t0, &[(0, Some("a")), (10, Some("b"))]);
        assert_eq!(
            log.dominant(t0 + secs(20), secs(20), ZERO, NOTHING),
            Some(app("b"))
        );
    }

    #[test]
    fn repeats_are_not_stored_but_time_still_counts() {
        let t0 = Instant::now();
        let mut log = ActivityLog::new();
        for s in 0..10 {
            log.record(t0 + secs(s), Some(app("game")));
        }
        assert_eq!(log.len(), 1);
        assert_eq!(
            log.dominant(t0 + secs(10), secs(10), secs(10), NOTHING),
            Some(app("game"))
        );
    }

    #[test]
    fn the_log_forgets_what_is_older_than_its_span_but_keeps_what_covers_it() {
        let t0 = Instant::now();
        let mut log = ActivityLog::new();
        // Alternate every minute for two hours.
        for minute in 0..120u64 {
            log.record(
                t0 + secs(minute * 60),
                Some(app(if minute % 2 == 0 { "a" } else { "b" })),
            );
        }
        assert!(
            log.len() <= (LOG_SPAN.as_secs() / 60) as usize + 2,
            "{}",
            log.len()
        );
        // The newest minutes are still answered correctly.
        let now = t0 + secs(120 * 60);
        assert_eq!(log.dominant(now, secs(60), ZERO, NOTHING), Some(app("b")));
    }

    #[test]
    fn a_window_longer_than_the_clock_history_does_not_panic() {
        let t0 = Instant::now();
        let log = log_of(t0, &[(0, Some("game"))]);
        let huge = Duration::from_secs(u64::MAX / 4);
        assert_eq!(
            log.dominant(t0 + secs(5), huge, ZERO, NOTHING),
            Some(app("game"))
        );
    }

    #[test]
    fn clearing_forgets_everything() {
        let t0 = Instant::now();
        let mut log = log_of(t0, &[(0, Some("game"))]);
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.dominant(t0 + secs(5), secs(5), ZERO, NOTHING), None);
    }

    #[test]
    fn the_default_ignore_list_covers_ourselves_and_the_shells() {
        for id in [
            "com.clipforge.app",
            "clipforge-app",
            "ClipForge.exe",
            "clipforge-app.exe",
            "com.apple.finder",
            "com.apple.dock",
            "explorer.exe",
            "Explorer.EXE",
            "searchhost.exe",
        ] {
            assert!(default_ignore(&app(id)), "{id} should be ignored");
        }
        for id in [
            "com.valve.steam",
            "valorant-win64-shipping.exe",
            "com.apple.safari",
            "code",
        ] {
            assert!(!default_ignore(&app(id)), "{id} should not be ignored");
        }
    }

    /// A source that walks through a script, then repeats its last entry.
    struct Script {
        steps: Vec<Option<AppInfo>>,
        next: usize,
        calls: Arc<Mutex<usize>>,
    }

    impl ForegroundSource for Script {
        fn foreground(&mut self) -> Option<AppInfo> {
            *self.calls.lock().unwrap() += 1;
            let step = self.steps[self.next.min(self.steps.len() - 1)].clone();
            self.next += 1;
            step
        }
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_tracker_samples_into_the_log() {
        let log = Arc::new(Mutex::new(ActivityLog::new()));
        let calls = Arc::new(Mutex::new(0));
        let tracker = ActivityTracker::start(
            Box::new(Script {
                steps: vec![Some(app("one")), Some(app("two"))],
                next: 0,
                calls: Arc::clone(&calls),
            }),
            Arc::clone(&log),
            Arc::new(AtomicBool::new(true)),
            Duration::from_millis(5),
        );
        wait_for("both apps to be sampled", || log.lock().unwrap().len() >= 2);
        drop(tracker);
        let log = log.lock().unwrap();
        assert_eq!(log.samples[0].1, Some(app("one")));
        assert_eq!(log.samples[1].1, Some(app("two")));
    }

    #[test]
    fn a_disabled_tracker_looks_at_nothing_and_forgets() {
        let log = Arc::new(Mutex::new(ActivityLog::new()));
        log.lock()
            .unwrap()
            .record(Instant::now(), Some(app("stale")));
        let calls = Arc::new(Mutex::new(0));
        let enabled = Arc::new(AtomicBool::new(false));
        let tracker = ActivityTracker::start(
            Box::new(Script {
                steps: vec![Some(app("one"))],
                next: 0,
                calls: Arc::clone(&calls),
            }),
            Arc::clone(&log),
            Arc::clone(&enabled),
            Duration::from_millis(5),
        );
        wait_for("the stale sample to be dropped", || {
            log.lock().unwrap().is_empty()
        });
        assert_eq!(
            *calls.lock().unwrap(),
            0,
            "never asked the OS while disabled"
        );

        // Switching it on makes it start looking.
        enabled.store(true, Ordering::Relaxed);
        wait_for("the first sample", || !log.lock().unwrap().is_empty());
        drop(tracker);
    }

    #[test]
    fn dropping_the_tracker_stops_the_thread_promptly() {
        let log = Arc::new(Mutex::new(ActivityLog::new()));
        let tracker = ActivityTracker::start(
            Box::new(Script {
                steps: vec![None],
                next: 0,
                calls: Arc::new(Mutex::new(0)),
            }),
            log,
            Arc::new(AtomicBool::new(true)),
            Duration::from_secs(60), // would sleep a minute if not woken
        );
        let started = Instant::now();
        drop(tracker);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
