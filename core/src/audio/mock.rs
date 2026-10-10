//! Deterministic [`AudioEncoder`] test double. Never used in production —
//! real encoders are the OS ones (AudioToolbox, Media Foundation).

use std::time::Duration;

use super::{
    frames_to_duration, AudioChunk, AudioConfig, AudioEncoder, AudioError, AudioPacket,
    AAC_FRAME_SAMPLES,
};

/// Emits one packet per 1024 input frames, like AAC does, with no encoder
/// delay. Each payload is `PACKET_BYTES` long and starts with the packet
/// index (little-endian `u64`), so tests can tell packets apart.
pub struct MockAacEncoder {
    config: AudioConfig,
    pending: Vec<f32>,
    /// Timestamp of the first sample in `pending` (valid while non-empty).
    pending_ts: Duration,
    next_index: u64,
    base: Option<Duration>,
    finished: bool,
}

pub const PACKET_BYTES: usize = 342; // ≈ 128 kb/s at 48 kHz

impl MockAacEncoder {
    pub fn new(config: AudioConfig) -> Self {
        Self {
            config,
            pending: Vec::new(),
            pending_ts: Duration::ZERO,
            next_index: 0,
            base: None,
            finished: false,
        }
    }

    fn drain(&mut self, flush: bool) -> Vec<AudioPacket> {
        let per_packet = AAC_FRAME_SAMPLES as usize * usize::from(self.config.channels);
        let mut out = Vec::new();
        while self.pending.len() >= per_packet || (flush && !self.pending.is_empty()) {
            let take = per_packet.min(self.pending.len());
            self.pending.drain(..take);
            let base = self.base.expect("base set with the first chunk");
            let pts = base
                + frames_to_duration(
                    self.next_index * u64::from(AAC_FRAME_SAMPLES),
                    self.config.sample_rate,
                );
            let mut data = vec![0u8; PACKET_BYTES];
            data[..8].copy_from_slice(&self.next_index.to_le_bytes());
            out.push(AudioPacket {
                data,
                pts,
                duration: frames_to_duration(u64::from(AAC_FRAME_SAMPLES), self.config.sample_rate),
            });
            self.next_index += 1;
        }
        out
    }
}

impl AudioEncoder for MockAacEncoder {
    fn encode(&mut self, chunk: &AudioChunk) -> Result<Vec<AudioPacket>, AudioError> {
        if self.finished {
            return Err(AudioError::Backend("encode after finish".into()));
        }
        if chunk.sample_rate != self.config.sample_rate || chunk.channels != self.config.channels {
            return Err(AudioError::BadChunk(format!(
                "{} Hz x{} chunk for {} Hz x{} encoder",
                chunk.sample_rate, chunk.channels, self.config.sample_rate, self.config.channels
            )));
        }
        if self.base.is_none() {
            self.base = Some(chunk.timestamp);
            self.pending_ts = chunk.timestamp;
        }
        self.pending.extend_from_slice(&chunk.samples);
        Ok(self.drain(false))
    }

    fn finish(&mut self) -> Result<Vec<AudioPacket>, AudioError> {
        self.finished = true;
        Ok(self.drain(true))
    }

    fn resync(&mut self) {
        self.pending.clear();
        self.base = None;
        self.next_index = 0;
    }

    fn config(&self) -> &AudioConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(frames: usize, ts_ms: u64) -> AudioChunk {
        AudioChunk {
            samples: vec![0.0; frames * 2],
            channels: 2,
            sample_rate: 48_000,
            timestamp: Duration::from_millis(ts_ms),
        }
    }

    #[test]
    fn emits_one_packet_per_1024_frames_regardless_of_chunking() {
        let mut enc = MockAacEncoder::new(AudioConfig::default());
        let mut packets = Vec::new();
        // 10 chunks of 480 frames = 4800 frames = 4 full packets + 704 left.
        for i in 0..10 {
            packets.extend(enc.encode(&chunk(480, i * 10)).unwrap());
        }
        assert_eq!(packets.len(), 4);
        packets.extend(enc.finish().unwrap());
        assert_eq!(packets.len(), 5, "finish flushes the partial packet");
    }

    #[test]
    fn pts_follow_the_sample_grid_from_the_first_chunk() {
        let mut enc = MockAacEncoder::new(AudioConfig::default());
        let packets = enc.encode(&chunk(4096, 250)).unwrap();
        assert_eq!(packets.len(), 4);
        for (k, p) in packets.iter().enumerate() {
            let expected = Duration::from_millis(250) + frames_to_duration(k as u64 * 1024, 48_000);
            assert_eq!(p.pts, expected);
            assert_eq!(p.duration, frames_to_duration(1024, 48_000));
        }
    }

    #[test]
    fn rejects_mismatched_chunks() {
        let mut enc = MockAacEncoder::new(AudioConfig::default());
        let mut bad = chunk(1024, 0);
        bad.sample_rate = 44_100;
        assert!(matches!(enc.encode(&bad), Err(AudioError::BadChunk(_))));
    }

    #[test]
    fn resync_restarts_the_timeline() {
        let mut enc = MockAacEncoder::new(AudioConfig::default());
        enc.encode(&chunk(2048, 0)).unwrap();
        enc.resync();
        let packets = enc.encode(&chunk(1024, 60_000)).unwrap();
        assert_eq!(packets[0].pts, Duration::from_secs(60));
    }

    #[test]
    fn encode_after_finish_is_an_error() {
        let mut enc = MockAacEncoder::new(AudioConfig::default());
        enc.finish().unwrap();
        assert!(enc.encode(&chunk(1024, 0)).is_err());
    }
}
