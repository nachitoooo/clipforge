//! App-side facade over the core engine: one shared object that the tray,
//! the global hotkey, and (later) the UI all drive.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clipforge_core::buffer::BufferConfig;
use clipforge_core::clip::{ClipService, SavedClip};
use clipforge_core::engine::CaptureEngine;

/// Default clip length saved by the hotkey (configurable UI comes in M3).
pub const DEFAULT_CLIP_SECONDS: u64 = 30;

pub struct Recorder {
    engine: Mutex<CaptureEngine>,
    clips: Arc<ClipService>,
}

impl Recorder {
    pub fn new(clips_dir: PathBuf) -> Self {
        let engine = CaptureEngine::new(BufferConfig::default());
        let clips = Arc::new(ClipService::new(engine.buffer(), clips_dir));
        Self {
            engine: Mutex::new(engine),
            clips,
        }
    }

    pub fn is_running(&self) -> bool {
        self.engine.lock().expect("engine lock").is_running()
    }

    pub fn clips_dir(&self) -> PathBuf {
        self.clips.clips_dir().to_path_buf()
    }

    /// Start the replay buffer (real capture + hardware encoder).
    #[cfg(target_os = "macos")]
    pub fn start(&self) -> Result<(), String> {
        use clipforge_core::audio::aac_audiotoolbox::AudioToolboxAacEncoder;
        use clipforge_core::audio::{AudioConfig, AudioEncoder};
        use clipforge_core::capture::macos::{ScreenCaptureConfig, ScreenCaptureKitSource};
        use clipforge_core::encode::videotoolbox::VideoToolboxEncoder;
        use clipforge_core::encode::EncoderConfig;

        // Sound is best effort: without an AAC encoder the clips are silent
        // instead of the whole buffer refusing to start.
        let audio_encoder: Option<Box<dyn AudioEncoder>> =
            match AudioToolboxAacEncoder::new(AudioConfig::default()) {
                Ok(encoder) => Some(Box::new(encoder)),
                Err(e) => {
                    eprintln!("clipforge: AAC encoder unavailable, clips will be silent: {e}");
                    None
                }
            };

        let config = EncoderConfig::default(); // documented 1080p60 defaults
        let source = ScreenCaptureKitSource::new(ScreenCaptureConfig {
            width: Some(config.width),
            height: Some(config.height),
            fps: config.fps,
            show_cursor: true,
            capture_audio: audio_encoder.is_some(),
        });
        let encoder = VideoToolboxEncoder::new(config).map_err(|e| e.to_string())?;
        self.engine
            .lock()
            .expect("engine lock")
            .start_with_audio(
                Box::new(source),
                Box::new(encoder),
                audio_encoder,
                Arc::clone(&self.clips),
            )
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
    }

    pub fn save_clip(&self, seconds: u64) -> Result<SavedClip, String> {
        self.clips
            .save_last(Duration::from_secs(seconds))
            .map_err(|e| e.to_string())
    }
}
