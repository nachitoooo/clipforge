//! Input level metering for the microphone VU bar in Settings.
//!
//! The audio thread calls [`LevelTap::record`] on every chunk; the UI side
//! polls [`LevelTap::take_peak`] ~20 times a second. Both are lock-free, so
//! metering never stalls capture.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Anything quieter than this (≈ -80 dBFS) counts as digital silence. A real
/// microphone always has a noise floor above it; a permission-denied one
/// delivers exact zeros.
pub const SIGNAL_THRESHOLD: f32 = 1e-4;

/// Largest absolute sample.
pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[derive(Debug, Default)]
pub struct LevelTap {
    /// Peak since the last `take_peak`, as `f32` bits. Non-negative floats
    /// order the same as their bit patterns, so `fetch_max` works directly.
    peak_bits: AtomicU32,
    heard: AtomicBool,
}

impl LevelTap {
    pub fn record(&self, samples: &[f32]) {
        let p = peak(samples);
        self.peak_bits.fetch_max(p.to_bits(), Ordering::Relaxed);
        if p > SIGNAL_THRESHOLD {
            self.heard.store(true, Ordering::Relaxed);
        }
    }

    /// Peak since the previous call (0.0 if nothing arrived), then reset.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak_bits.swap(0, Ordering::Relaxed))
    }

    /// Whether anything above digital silence has ever been recorded.
    pub fn heard_signal(&self) -> bool {
        self.heard.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        self.peak_bits.store(0, Ordering::Relaxed);
        self.heard.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn peak_is_the_largest_absolute_sample() {
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(peak(&[0.1, -0.7, 0.3]), 0.7);
    }

    #[test]
    fn take_returns_the_peak_since_the_last_take() {
        let tap = LevelTap::default();
        tap.record(&[0.2, -0.1]);
        tap.record(&[0.6]);
        tap.record(&[0.3]);
        assert_eq!(tap.take_peak(), 0.6);
        assert_eq!(tap.take_peak(), 0.0, "reset after take");
        tap.record(&[0.05]);
        assert_eq!(tap.take_peak(), 0.05);
    }

    #[test]
    fn silence_is_not_a_signal() {
        let tap = LevelTap::default();
        tap.record(&[0.0; 480]);
        tap.record(&[SIGNAL_THRESHOLD / 2.0]);
        assert!(!tap.heard_signal());
        tap.record(&[0.01]);
        assert!(tap.heard_signal());
        tap.reset();
        assert!(!tap.heard_signal());
        assert_eq!(tap.take_peak(), 0.0);
    }

    #[test]
    fn recording_from_many_threads_keeps_the_maximum() {
        let tap = Arc::new(LevelTap::default());
        let handles: Vec<_> = (1..=8)
            .map(|i| {
                let tap = Arc::clone(&tap);
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        tap.record(&[i as f32 / 10.0]);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(tap.take_peak(), 0.8);
    }
}
