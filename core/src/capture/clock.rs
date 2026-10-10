//! The clock video and audio share.
//!
//! Frames and audio chunks arrive stamped with the OS's own media clock
//! (host time on macOS, QPC on Windows). ClipForge re-bases both onto one
//! epoch — the first video frame — so a timestamp means the same instant in
//! both streams. [`SharedEpoch`] is that epoch: set once by whichever stream
//! sees the first video frame, read by everyone.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const UNSET: i64 = i64::MIN;

#[derive(Clone)]
pub struct SharedEpoch(Arc<AtomicI64>);

impl Default for SharedEpoch {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedEpoch {
    pub fn new() -> Self {
        Self(Arc::new(AtomicI64::new(UNSET)))
    }

    /// Claim `now_ns` as the epoch unless one is already set; returns the
    /// epoch in force either way. Call this from the *video* path only.
    pub fn get_or_set(&self, now_ns: i64) -> i64 {
        match self
            .0
            .compare_exchange(UNSET, now_ns, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => now_ns,
            Err(existing) => existing,
        }
    }

    pub fn get(&self) -> Option<i64> {
        match self.0.load(Ordering::SeqCst) {
            UNSET => None,
            ns => Some(ns),
        }
    }

    /// `ns_on_media_clock` relative to the epoch, or `None` when the epoch
    /// isn't set yet or the instant precedes it (audio captured before the
    /// first video frame has no place in the clip).
    pub fn relative(&self, ns_on_media_clock: i64) -> Option<Duration> {
        let rel = ns_on_media_clock.checked_sub(self.get()?)?;
        u64::try_from(rel).ok().map(Duration::from_nanos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_epoch_yields_nothing() {
        let epoch = SharedEpoch::new();
        assert_eq!(epoch.get(), None);
        assert_eq!(epoch.relative(1_000), None);
    }

    #[test]
    fn first_setter_wins_and_clones_share_it() {
        let epoch = SharedEpoch::new();
        let other = epoch.clone();
        assert_eq!(epoch.get_or_set(5_000), 5_000);
        assert_eq!(other.get_or_set(9_000), 5_000, "second claim loses");
        assert_eq!(other.get(), Some(5_000));
    }

    #[test]
    fn relative_rejects_instants_before_the_epoch() {
        let epoch = SharedEpoch::new();
        epoch.get_or_set(10_000_000);
        assert_eq!(epoch.relative(10_000_000), Some(Duration::ZERO));
        assert_eq!(
            epoch.relative(12_500_000),
            Some(Duration::from_micros(2_500))
        );
        assert_eq!(epoch.relative(9_999_999), None);
    }

    #[test]
    fn negative_epochs_are_legal() {
        let epoch = SharedEpoch::new();
        epoch.get_or_set(-50);
        assert_eq!(epoch.relative(-10), Some(Duration::from_nanos(40)));
    }
}
