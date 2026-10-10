//! Format conversion for microphone input: any device rate/channel count in,
//! 48 kHz mono out (the format ClipForge encodes voice in).
//!
//! Linear interpolation is plenty for speech (the error at 440 Hz from
//! 44.1 kHz is ~2·10⁻⁴ of full scale) and keeps the audio thread free of
//! dependencies and allocations beyond the output buffer.

/// Average interleaved frames down to one channel. Mono passes through.
pub fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Stateful linear resampler for interleaved `f32`.
///
/// State carried between calls (the fractional read position and the last
/// input frame) makes the output independent of how the input was chunked:
/// feeding one big buffer or many small ones yields the same samples.
pub struct LinearResampler {
    channels: usize,
    /// Input frames consumed per output frame (`in_rate / out_rate`).
    step: f64,
    /// Read position in input frames, relative to the start of the next
    /// input chunk; `-1.0` means "the previous chunk's last frame".
    pos: f64,
    prev: Vec<f32>,
    identity: bool,
}

impl LinearResampler {
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            channels,
            step: f64::from(in_rate) / f64::from(out_rate.max(1)),
            pos: 0.0,
            prev: vec![0.0; channels],
            identity: in_rate == out_rate,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.identity {
            return input.to_vec();
        }
        let ch = self.channels;
        let frames = input.len() / ch;
        if frames == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(((frames as f64 / self.step) as usize + 2) * ch);
        let last = (frames - 1) as f64;
        while self.pos < last {
            let base = self.pos.floor();
            let frac = (self.pos - base) as f32;
            let k = base as isize;
            for c in 0..ch {
                let a = if k < 0 {
                    self.prev[c]
                } else {
                    input[k as usize * ch + c]
                };
                let b = input[(k + 1) as usize * ch + c];
                out.push(a + (b - a) * frac);
            }
            self.pos += self.step;
        }
        self.pos -= frames as f64;
        self.prev
            .copy_from_slice(&input[(frames - 1) * ch..frames * ch]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(rate: u32, freq: f32, frames: usize, offset: usize) -> Vec<f32> {
        (offset..offset + frames)
            .map(|n| 0.5 * (TAU * freq * n as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn to_mono_averages_channels() {
        assert_eq!(to_mono(&[1.0, 3.0, -1.0, -3.0], 2), [2.0, -2.0]);
        assert_eq!(to_mono(&[0.1, 0.2, 0.3], 1), [0.1, 0.2, 0.3]);
        // Three channels: plain mean; a torn last frame is ignored.
        assert_eq!(to_mono(&[3.0, 6.0, 9.0, 1.0], 3), [6.0]);
    }

    #[test]
    fn same_rate_is_a_passthrough() {
        let mut r = LinearResampler::new(48_000, 48_000, 1);
        let input = sine(48_000, 440.0, 480, 0);
        assert_eq!(r.process(&input), input);
    }

    #[test]
    fn upsampling_44k_to_48k_keeps_length_and_shape() {
        let mut r = LinearResampler::new(44_100, 48_000, 1);
        let mut out = Vec::new();
        for chunk in 0..100 {
            out.extend(r.process(&sine(44_100, 440.0, 441, chunk * 441))); // 10 ms chunks
        }
        // 1 s in → ~1 s out (one frame of interpolation lag at most).
        assert!(
            (47_995..=48_001).contains(&out.len()),
            "{} frames",
            out.len()
        );
        // Output sample j must be the 440 Hz sine at j / 48 kHz.
        let worst = out
            .iter()
            .enumerate()
            .map(|(j, s)| (s - 0.5 * (TAU * 440.0 * j as f32 / 48_000.0).sin()).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.002, "worst error {worst}");
    }

    #[test]
    fn upsampling_16k_triples_the_frames() {
        let mut r = LinearResampler::new(16_000, 48_000, 1);
        let out = r.process(&sine(16_000, 300.0, 1600, 0));
        assert!((4_795..=4_800).contains(&out.len()), "{} frames", out.len());
    }

    #[test]
    fn downsampling_96k_halves_the_frames() {
        let mut r = LinearResampler::new(96_000, 48_000, 1);
        let out = r.process(&sine(96_000, 300.0, 9_600, 0));
        assert!((4_798..=4_800).contains(&out.len()), "{} frames", out.len());
    }

    #[test]
    fn the_result_does_not_depend_on_how_the_input_is_chunked() {
        let input = sine(44_100, 523.0, 4_410, 0);
        let mut whole = LinearResampler::new(44_100, 48_000, 1);
        let reference = whole.process(&input);

        for chunk in [1usize, 7, 64, 441, 1000] {
            let mut r = LinearResampler::new(44_100, 48_000, 1);
            let mut out = Vec::new();
            for piece in input.chunks(chunk) {
                out.extend(r.process(piece));
            }
            assert_eq!(out.len(), reference.len(), "chunk size {chunk}");
            for (i, (a, b)) in out.iter().zip(&reference).enumerate() {
                assert!(
                    (a - b).abs() < 1e-5,
                    "chunk size {chunk}, sample {i}: {a} vs {b}"
                );
            }
        }
    }

    #[test]
    fn channels_stay_separate() {
        let mut r = LinearResampler::new(44_100, 48_000, 2);
        let stereo: Vec<f32> = (0..441).flat_map(|_| [1.0, -1.0]).collect();
        let out = r.process(&stereo);
        assert!(!out.is_empty());
        for frame in out.chunks_exact(2) {
            assert!((frame[0] - 1.0).abs() < 1e-6 && (frame[1] + 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn empty_input_is_harmless() {
        let mut r = LinearResampler::new(44_100, 48_000, 1);
        assert!(r.process(&[]).is_empty());
        assert!(!r.process(&sine(44_100, 100.0, 100, 0)).is_empty());
    }
}
