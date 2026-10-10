//! Synthetic audio source for tests: a sine tone delivered in 10 ms chunks
//! with exact timestamps — the audio counterpart of
//! [`crate::capture::test_pattern::TestPatternSource`].

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::{frames_to_duration, AudioChunk};
use crate::capture::{AudioSource, CaptureError};

const CHUNK_FRAMES: usize = 480; // 10 ms at 48 kHz
const CHANNEL_CAPACITY: usize = 64;

#[derive(Debug, Clone)]
pub struct TestToneConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub frequency: f32,
    /// Stop after this much audio; `None` streams until `stop()`.
    pub duration_limit: Option<Duration>,
    /// Pace to the wall clock (like a real device) instead of generating as
    /// fast as the consumer accepts.
    pub realtime: bool,
}

impl Default for TestToneConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            frequency: 440.0,
            duration_limit: None,
            realtime: false,
        }
    }
}

pub struct TestToneSource {
    config: TestToneConfig,
    running: Arc<AtomicBool>,
    generator: Option<std::thread::JoinHandle<()>>,
}

impl TestToneSource {
    pub fn new(config: TestToneConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            generator: None,
        }
    }

    /// The tone sample at absolute frame index `n`.
    pub fn sample_at(config: &TestToneConfig, n: u64) -> f32 {
        let phase =
            (n as f64 * f64::from(config.frequency) / f64::from(config.sample_rate)).fract();
        0.5 * (TAU * phase as f32).sin()
    }
}

impl AudioSource for TestToneSource {
    fn start(&mut self) -> Result<mpsc::Receiver<AudioChunk>, CaptureError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(CaptureError::AlreadyStarted);
        }
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let config = self.config.clone();
        let running = Arc::clone(&self.running);

        self.generator = Some(std::thread::spawn(move || {
            let epoch = std::time::Instant::now();
            let mut frame_index: u64 = 0;
            while running.load(Ordering::SeqCst) {
                let timestamp = frames_to_duration(frame_index, config.sample_rate);
                if config
                    .duration_limit
                    .is_some_and(|limit| timestamp >= limit)
                {
                    break;
                }
                if config.realtime {
                    if let Some(wait) = timestamp.checked_sub(epoch.elapsed()) {
                        std::thread::sleep(wait);
                    }
                }
                let mut samples = Vec::with_capacity(CHUNK_FRAMES * usize::from(config.channels));
                for i in 0..CHUNK_FRAMES as u64 {
                    let s = TestToneSource::sample_at(&config, frame_index + i);
                    samples.extend(std::iter::repeat_n(s, usize::from(config.channels)));
                }
                let chunk = AudioChunk {
                    samples,
                    channels: config.channels,
                    sample_rate: config.sample_rate,
                    timestamp,
                };
                if tx.blocking_send(chunk).is_err() {
                    break;
                }
                frame_index += CHUNK_FRAMES as u64;
            }
            // Dropping `tx` ends the stream.
        }));
        Ok(rx)
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.generator.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TestToneSource {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_exact_timestamps_and_stops_at_the_limit() {
        let mut source = TestToneSource::new(TestToneConfig {
            duration_limit: Some(Duration::from_millis(100)),
            ..Default::default()
        });
        let mut rx = source.start().unwrap();
        let mut chunks = Vec::new();
        while let Some(chunk) = rx.blocking_recv() {
            chunks.push(chunk);
        }
        assert_eq!(chunks.len(), 10);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.timestamp, Duration::from_millis(i as u64 * 10));
            assert_eq!(c.frames(), CHUNK_FRAMES);
        }
    }

    #[test]
    fn tone_is_a_sine_of_the_requested_frequency() {
        let config = TestToneConfig::default();
        // 440 Hz at 48 kHz: sample 0 is zero, a quarter period later is the peak.
        assert_eq!(TestToneSource::sample_at(&config, 0), 0.0);
        let quarter = (48_000.0f32 / 440.0 / 4.0).round() as u64;
        assert!(TestToneSource::sample_at(&config, quarter) > 0.49);
    }

    #[test]
    fn double_start_is_rejected() {
        let mut source = TestToneSource::new(TestToneConfig::default());
        let _rx = source.start().unwrap();
        assert!(matches!(source.start(), Err(CaptureError::AlreadyStarted)));
        source.stop();
    }
}
