//! Source → encoder pipeline tests using the synthetic TestPatternSource and
//! a mock encoder, per the testing rules in CLAUDE.md: the pipeline must be
//! verifiable without a real screen or a real hardware encoder.

use std::time::Duration;

use clipforge_core::capture::test_pattern::{TestPatternConfig, TestPatternSource};
use clipforge_core::capture::{FrameSource, RawFrame};
use clipforge_core::encode::{EncodeError, EncodedPacket, Encoder, EncoderConfig, ParameterSets};

/// Deterministic stand-in for a hardware encoder: emits one packet per frame,
/// keyframes on the configured GOP boundary, and mimics hardware pipelining
/// by holding each packet back until the next `encode`/`finish` call.
struct MockEncoder {
    config: EncoderConfig,
    pending: Option<EncodedPacket>,
    finished: bool,
    frames_in: u64,
}

impl MockEncoder {
    fn new(config: EncoderConfig) -> Self {
        Self {
            config,
            pending: None,
            finished: false,
            frames_in: 0,
        }
    }
}

impl Encoder for MockEncoder {
    fn encode(&mut self, frame: &RawFrame) -> Result<Vec<EncodedPacket>, EncodeError> {
        if self.finished {
            return Err(EncodeError::Backend("encode after finish".into()));
        }
        if frame.width != self.config.width || frame.height != self.config.height {
            return Err(EncodeError::BadFrame(format!(
                "{}x{} frame for {}x{} encoder",
                frame.width, frame.height, self.config.width, self.config.height
            )));
        }
        let packet = EncodedPacket {
            data: frame.frame_number.to_le_bytes().to_vec(),
            pts: frame.timestamp,
            dts: frame.timestamp,
            is_keyframe: self
                .frames_in
                .is_multiple_of(self.config.keyframe_interval as u64),
        };
        self.frames_in += 1;
        Ok(self.pending.replace(packet).into_iter().collect())
    }

    fn finish(&mut self) -> Result<Vec<EncodedPacket>, EncodeError> {
        self.finished = true;
        Ok(self.pending.take().into_iter().collect())
    }

    fn parameter_sets(&self) -> Option<ParameterSets> {
        (self.frames_in > 0).then(ParameterSets::default)
    }

    fn config(&self) -> &EncoderConfig {
        &self.config
    }
}

/// Run `frames` test-pattern frames through an encoder and collect packets.
async fn run_pipeline(frames: u64, config: EncoderConfig) -> Vec<EncodedPacket> {
    let mut source = TestPatternSource::new(TestPatternConfig {
        width: config.width,
        height: config.height,
        fps: config.fps,
        frame_limit: Some(frames),
        realtime: false,
    });
    let mut encoder = MockEncoder::new(config);
    let mut rx = source.start().expect("source starts");
    let mut packets = Vec::new();
    while let Some(frame) = rx.recv().await {
        packets.extend(encoder.encode(&frame).expect("encode"));
    }
    packets.extend(encoder.finish().expect("finish"));
    packets
}

fn small_config() -> EncoderConfig {
    EncoderConfig {
        width: 64,
        height: 48,
        fps: 60,
        keyframe_interval: 30,
        ..Default::default()
    }
}

#[tokio::test]
async fn pipeline_produces_one_packet_per_frame_with_monotonic_timestamps() {
    let packets = run_pipeline(200, small_config()).await;
    assert_eq!(packets.len(), 200, "no packet lost in pipelining");

    for pair in packets.windows(2) {
        assert!(
            pair[1].pts > pair[0].pts,
            "pts must strictly increase: {:?} then {:?}",
            pair[0].pts,
            pair[1].pts
        );
        assert!(pair[1].dts >= pair[0].dts, "dts must never go backwards");
    }
}

#[tokio::test]
async fn pipeline_timestamps_match_frame_grid() {
    let packets = run_pipeline(60, small_config()).await;
    // 60 frames at 60 fps: last pts is frame 59 at ~983 ms; spacing 1/60 s.
    let step = Duration::from_nanos(1_000_000_000 / 60);
    for (i, packet) in packets.iter().enumerate() {
        let expected = Duration::from_nanos(i as u64 * 1_000_000_000 / 60);
        assert_eq!(packet.pts, expected);
        if i > 0 {
            let delta = packet.pts - packets[i - 1].pts;
            // Integer-nanosecond grid: deltas are within 1 ns of 1/60 s.
            assert!(
                delta >= step - Duration::from_nanos(1) && delta <= step + Duration::from_nanos(1),
                "uneven frame spacing at packet {i}: {delta:?}"
            );
        }
    }
}

#[tokio::test]
async fn keyframes_land_on_gop_boundaries() {
    let packets = run_pipeline(100, small_config()).await;
    for (i, packet) in packets.iter().enumerate() {
        assert_eq!(
            packet.is_keyframe,
            i % 30 == 0,
            "packet {i} keyframe flag wrong"
        );
    }
}

#[tokio::test]
async fn encoder_rejects_mismatched_frames() {
    let mut encoder = MockEncoder::new(small_config());
    let mut source = TestPatternSource::new(TestPatternConfig {
        width: 32, // different from the encoder's 64x48
        height: 32,
        fps: 60,
        frame_limit: Some(1),
        realtime: false,
    });
    let mut rx = source.start().unwrap();
    let frame = rx.recv().await.unwrap();
    assert!(matches!(
        encoder.encode(&frame),
        Err(EncodeError::BadFrame(_))
    ));
}

#[tokio::test]
async fn parameter_sets_appear_only_after_first_frame() {
    let mut encoder = MockEncoder::new(small_config());
    assert!(encoder.parameter_sets().is_none());
    let mut source = TestPatternSource::new(TestPatternConfig {
        width: 64,
        height: 48,
        fps: 60,
        frame_limit: Some(1),
        realtime: false,
    });
    let mut rx = source.start().unwrap();
    let frame = rx.recv().await.unwrap();
    encoder.encode(&frame).unwrap();
    assert!(encoder.parameter_sets().is_some());
}
