//! App-side facade over the core engine: one shared object that the tray,
//! the global hotkey, and (later) the UI all drive.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clipforge_core::buffer::BufferConfig;
use clipforge_core::clip::{ClipService, SavedClip};
use clipforge_core::engine::CaptureEngine;
use serde::Serialize;

/// Default clip length saved by the hotkey (configurable UI comes in M3).
pub const DEFAULT_CLIP_SECONDS: u64 = 30;

/// What the UI shows about the replay buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BufferStatus {
    pub running: bool,
    /// Why the buffer isn't running, when the last start attempt failed.
    pub error: Option<String>,
}

pub struct Recorder {
    engine: Mutex<CaptureEngine>,
    clips: Arc<ClipService>,
    last_start_error: Mutex<Option<String>>,
}

impl Recorder {
    pub fn new(clips_dir: PathBuf) -> Self {
        let engine = CaptureEngine::new(BufferConfig::default());
        let clips = Arc::new(ClipService::new(engine.buffer(), clips_dir));
        Self {
            engine: Mutex::new(engine),
            clips,
            last_start_error: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.engine.lock().expect("engine lock").is_running()
    }

    pub fn status(&self) -> BufferStatus {
        BufferStatus {
            running: self.is_running(),
            error: self.last_start_error.lock().expect("error lock").clone(),
        }
    }

    /// Remember the outcome of a start attempt so a UI that loads later can
    /// still explain why the buffer is stopped.
    pub fn record_start_result(&self, result: &Result<(), String>) {
        *self.last_start_error.lock().expect("error lock") = result.as_ref().err().cloned();
    }

    pub fn clips_dir(&self) -> PathBuf {
        self.clips.clips_dir().to_path_buf()
    }

    /// Start the replay buffer (real capture + hardware encoder).
    #[cfg(target_os = "macos")]
    pub fn start(&self) -> Result<(), String> {
        use clipforge_core::capture::macos::{ScreenCaptureConfig, ScreenCaptureKitSource};
        use clipforge_core::encode::videotoolbox::VideoToolboxEncoder;
        use clipforge_core::encode::EncoderConfig;

        let config = EncoderConfig::default(); // documented 1080p60 defaults
        let source = ScreenCaptureKitSource::new(ScreenCaptureConfig {
            width: Some(config.width),
            height: Some(config.height),
            fps: config.fps,
            show_cursor: true,
        });
        let encoder = VideoToolboxEncoder::new(config).map_err(|e| e.to_string())?;
        self.engine
            .lock()
            .expect("engine lock")
            .start(Box::new(source), Box::new(encoder), Arc::clone(&self.clips))
            .map_err(|e| e.to_string())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn start(&self) -> Result<(), String> {
        Err("la captura en este sistema operativo llega con F-2.4 (Windows)".into())
    }

    /// Stop capturing; the buffered window survives so a clip can still be
    /// saved right after stopping.
    pub fn stop(&self) {
        self.engine.lock().expect("engine lock").stop();
        self.record_start_result(&Ok(())); // a deliberate stop isn't an error
    }

    pub fn save_clip(&self, seconds: u64) -> Result<SavedClip, String> {
        self.clips
            .save_last(Duration::from_secs(seconds))
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder() -> Recorder {
        Recorder::new(std::env::temp_dir().join("clipforge-recorder-test"))
    }

    #[test]
    fn fresh_recorder_is_stopped_without_error() {
        assert_eq!(
            recorder().status(),
            BufferStatus {
                running: false,
                error: None
            }
        );
    }

    #[test]
    fn failed_start_is_remembered_until_the_next_attempt_succeeds() {
        let recorder = recorder();
        recorder.record_start_result(&Err("permiso denegado".into()));
        let status = recorder.status();
        assert!(!status.running);
        assert_eq!(status.error.as_deref(), Some("permiso denegado"));

        recorder.record_start_result(&Ok(()));
        assert_eq!(recorder.status().error, None);
    }

    #[test]
    fn stopping_clears_a_previous_error() {
        let recorder = recorder();
        recorder.record_start_result(&Err("boom".into()));
        recorder.stop();
        assert_eq!(recorder.status().error, None);
    }
}
