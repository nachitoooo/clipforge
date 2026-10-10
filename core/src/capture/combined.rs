//! Glue for platforms where video and audio come from separate OS APIs
//! (Windows: Graphics Capture + WASAPI loopback). On macOS ScreenCaptureKit
//! delivers both from one stream and implements `take_audio` itself.

use tokio::sync::mpsc;

use super::{AudioSource, CaptureError, CaptureStats, FrameSource, RawFrame};
use crate::audio::AudioChunk;

/// A [`FrameSource`] that also exposes an [`AudioSource`]'s stream through
/// `take_audio`. Audio trouble never takes the video down: if the audio
/// source fails to start, the capture runs without sound.
pub struct WithAudio<V: FrameSource, A: AudioSource> {
    // Declared first on purpose: fields drop in order, and an audio source
    // that is blocked sending into a full channel only wakes up once the
    // receiver is gone — which must happen before the source's own drop
    // tries to join its thread.
    audio_rx: Option<mpsc::Receiver<AudioChunk>>,
    video: V,
    audio: A,
    audio_error: Option<CaptureError>,
}

impl<V: FrameSource, A: AudioSource> WithAudio<V, A> {
    pub fn new(video: V, audio: A) -> Self {
        Self {
            audio_rx: None,
            video,
            audio,
            audio_error: None,
        }
    }

    /// Why audio isn't being captured, if starting it failed.
    pub fn audio_error(&self) -> Option<&CaptureError> {
        self.audio_error.as_ref()
    }
}

impl<V: FrameSource, A: AudioSource> FrameSource for WithAudio<V, A> {
    fn start(&mut self) -> Result<mpsc::Receiver<RawFrame>, CaptureError> {
        let frames = self.video.start()?;
        match self.audio.start() {
            Ok(rx) => {
                self.audio_rx = Some(rx);
                self.audio_error = None;
            }
            Err(e) => {
                eprintln!("clipforge: audio capture unavailable, recording video only: {e}");
                self.audio_error = Some(e);
            }
        }
        Ok(frames)
    }

    fn stop(&mut self) {
        // Nobody is listening any more; unblock a producer stuck on a full
        // channel before asking it to stop.
        self.audio_rx = None;
        self.audio.stop();
        self.video.stop();
    }

    fn stats(&self) -> CaptureStats {
        self.video.stats()
    }

    fn take_audio(&mut self) -> Option<mpsc::Receiver<AudioChunk>> {
        self.audio_rx.take()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::audio::test_tone::{TestToneConfig, TestToneSource};
    use crate::capture::test_pattern::{TestPatternConfig, TestPatternSource};

    fn video() -> TestPatternSource {
        TestPatternSource::new(TestPatternConfig {
            width: 64,
            height: 48,
            frame_limit: Some(3),
            ..Default::default()
        })
    }

    struct FailingAudio;
    impl AudioSource for FailingAudio {
        fn start(&mut self) -> Result<mpsc::Receiver<AudioChunk>, CaptureError> {
            Err(CaptureError::Backend("no loopback device".into()))
        }
        fn stop(&mut self) {}
    }

    #[test]
    fn exposes_the_audio_stream_once() {
        let audio = TestToneSource::new(TestToneConfig {
            duration_limit: Some(Duration::from_millis(30)),
            ..Default::default()
        });
        let mut source = WithAudio::new(video(), audio);
        let mut frames = source.start().unwrap();
        let mut audio_rx = source.take_audio().expect("audio available after start");
        assert!(source.take_audio().is_none(), "can only be taken once");

        let mut frame_count = 0;
        while frames.blocking_recv().is_some() {
            frame_count += 1;
        }
        let mut chunk_count = 0;
        while audio_rx.blocking_recv().is_some() {
            chunk_count += 1;
        }
        assert_eq!(frame_count, 3);
        assert_eq!(chunk_count, 3);
        source.stop();
    }

    #[test]
    fn a_failing_audio_source_leaves_video_running() {
        let mut source = WithAudio::new(video(), FailingAudio);
        let mut frames = source.start().expect("video must still start");
        assert!(source.take_audio().is_none());
        assert!(matches!(
            source.audio_error(),
            Some(CaptureError::Backend(_))
        ));
        let mut n = 0;
        while frames.blocking_recv().is_some() {
            n += 1;
        }
        assert_eq!(n, 3);
        source.stop();
    }

    #[test]
    fn video_failure_is_still_an_error() {
        let bad_video = TestPatternSource::new(TestPatternConfig {
            fps: 0,
            ..Default::default()
        });
        let mut source = WithAudio::new(bad_video, TestToneSource::new(TestToneConfig::default()));
        assert!(source.start().is_err());
    }
}
