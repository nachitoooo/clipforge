//! Deterministic [`Encoder`] test double, shared by core tests and the app
//! crate's tests. Never used in production — real encoders are hardware-only.

use super::{EncodeError, EncodedPacket, Encoder, EncoderConfig, ParameterSets};
use crate::capture::RawFrame;

/// Emits one packet per frame with keyframes on the configured GOP boundary,
/// holding each packet back one call to mimic hardware pipelining. Packet
/// payloads are a valid AVCC shell carrying the frame number.
pub struct MockEncoder {
    config: EncoderConfig,
    pending: Option<EncodedPacket>,
    finished: bool,
    frames_in: u64,
}

impl MockEncoder {
    pub fn new(config: EncoderConfig) -> Self {
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
        let is_keyframe = self
            .frames_in
            .is_multiple_of(self.config.keyframe_interval as u64);
        // AVCC shell: one NAL, length-prefixed, IDR (0x65) or non-IDR (0x41),
        // then the frame number for traceability.
        let mut data = 9u32.to_be_bytes().to_vec();
        data.push(if is_keyframe { 0x65 } else { 0x41 });
        data.extend_from_slice(&frame.frame_number.to_le_bytes());
        let packet = EncodedPacket {
            data,
            pts: frame.timestamp,
            dts: frame.timestamp,
            is_keyframe,
        };
        self.frames_in += 1;
        Ok(self.pending.replace(packet).into_iter().collect())
    }

    fn finish(&mut self) -> Result<Vec<EncodedPacket>, EncodeError> {
        self.finished = true;
        Ok(self.pending.take().into_iter().collect())
    }

    fn parameter_sets(&self) -> Option<ParameterSets> {
        (self.frames_in > 0).then(|| ParameterSets {
            sps: vec![vec![0x67, 0x42, 0xc0, 0x28, 0x96, 0x54, 0x03, 0xc0]],
            pps: vec![vec![0x68, 0xce, 0x3c, 0x80]],
        })
    }

    fn config(&self) -> &EncoderConfig {
        &self.config
    }
}
