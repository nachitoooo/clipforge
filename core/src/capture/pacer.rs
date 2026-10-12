//! Frame pacing for sources that can deliver faster than the target frame
//! rate: a 144 Hz monitor feeding a 60 fps recording. Pure code, tested on
//! every OS.

/// Slack allowed before a frame counts as "due", so that frames a hair early
/// are not thrown away and the output keeps a steady cadence.
const TOLERANCE_NS: i64 = 2_000_000;

/// Decides which of a fast stream of frames to keep so that about `fps` per
/// second survive, evenly spaced. Frames are never *added*: a screen that
/// doesn't change simply produces fewer.
pub struct Pacer {
    interval_ns: i64,
    next_due: i64,
}

impl Pacer {
    pub fn new(fps: u32) -> Self {
        Self {
            interval_ns: 1_000_000_000 / i64::from(fps.max(1)),
            next_due: 0,
        }
    }

    /// Whether the frame stamped `ns` (nanoseconds since the first frame,
    /// strictly increasing) should be kept.
    pub fn accept(&mut self, ns: i64) -> bool {
        if ns + TOLERANCE_NS < self.next_due {
            return false;
        }
        // Stay on the grid while the stream is steady; after a long pause
        // (a screen that did not change) start a fresh one from here.
        self.next_due = (self.next_due + self.interval_ns).max(ns + self.interval_ns / 2);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(fps: u32, source_hz: u32, seconds: u32) -> Vec<i64> {
        let mut pacer = Pacer::new(fps);
        let step = 1_000_000_000 / i64::from(source_hz);
        (0..i64::from(source_hz * seconds))
            .map(|n| n * step)
            .filter(|&ns| pacer.accept(ns))
            .collect()
    }

    #[test]
    fn a_source_at_the_target_rate_loses_nothing() {
        assert_eq!(kept(60, 60, 2).len(), 120);
        assert_eq!(kept(30, 30, 2).len(), 60);
    }

    #[test]
    fn a_slower_source_is_left_alone() {
        assert_eq!(kept(60, 30, 2).len(), 60);
    }

    #[test]
    fn a_faster_source_is_thinned_to_about_the_target() {
        for (fps, hz) in [(60, 144), (60, 120), (60, 240), (30, 60), (30, 144)] {
            let n = kept(fps, hz, 3).len() as f64;
            let expected = f64::from(fps * 3);
            assert!(
                (n - expected).abs() <= expected * 0.06,
                "{hz} Hz -> {fps} fps kept {n}, wanted about {expected}"
            );
        }
    }

    #[test]
    fn the_frames_kept_are_evenly_spaced() {
        let times = kept(60, 144, 3);
        let gaps: Vec<i64> = times.windows(2).map(|w| w[1] - w[0]).collect();
        let (min, max) = (gaps.iter().min().unwrap(), gaps.iter().max().unwrap());
        // one source frame of jitter (6.9 ms) around the 16.7 ms target
        assert!(*min >= 10_000_000 && *max <= 24_000_000, "{min} .. {max}");
    }

    #[test]
    fn the_first_frame_is_always_kept() {
        assert!(Pacer::new(60).accept(0));
        assert!(Pacer::new(60).accept(123_000_000));
    }

    #[test]
    fn a_long_pause_starts_a_fresh_grid_instead_of_bursting() {
        let mut pacer = Pacer::new(60);
        assert!(pacer.accept(0));
        // five seconds of nothing, then a screen that updates at 144 Hz
        let start = 5_000_000_000i64;
        let step = 6_944_444i64;
        let kept = (0..30).filter(|n| pacer.accept(start + n * step)).count();
        assert!((11..=14).contains(&kept), "kept {kept} of 30 in ~208 ms");
    }

    #[test]
    fn a_zero_rate_does_not_divide_by_zero() {
        let mut pacer = Pacer::new(0);
        assert!(pacer.accept(0));
    }
}
