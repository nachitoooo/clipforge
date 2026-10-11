//! App-side facade over the core engine: one shared object that the tray,
//! the global hotkey, and (later) the UI all drive.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clipforge_core::activity::{
    default_ignore, system_source, ActivityLog, ActivityTracker, SAMPLE_INTERVAL,
};

use clipforge_core::audio::mic::{
    list_input_devices, mic_audio_config, CpalMicSource, MicControls,
};
use clipforge_core::audio::{AudioConfig, AudioEncoder, MicMode as CoreMicMode};
use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::AudioSource;
use clipforge_core::clip::{ClipService, SavedClip};
use clipforge_core::engine::CaptureEngine;
use serde::Serialize;

use crate::mic::{
    gain_from_percent, plan, status as mic_status_from, AppliedMic, MicAction, MicDevice, MicFacts,
    MicStatus,
};
use crate::settings::{MicMode, MicrophoneSettings, SettingsStore};

/// Default clip length saved by the hotkey (configurable UI comes in M3).
pub const DEFAULT_CLIP_SECONDS: u64 = 30;

/// What the UI shows about the replay buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BufferStatus {
    pub running: bool,
    /// Why the buffer isn't running, when the last start attempt failed.
    pub error: Option<String>,
}

/// What the recorder knows about the microphone it attached.
#[derive(Default)]
struct MicRuntime {
    applied: Option<AppliedMic>,
    /// Why the last attach attempt failed.
    attach_error: Option<String>,
    since: Option<Instant>,
}

pub struct Recorder {
    engine: Mutex<CaptureEngine>,
    clips: Arc<ClipService>,
    last_start_error: Mutex<Option<String>>,
    settings: Arc<SettingsStore>,
    mic_controls: Arc<MicControls>,
    mic: Mutex<MicRuntime>,
    /// Serializes `apply_microphone`: the UI, the mute hotkey and the buffer
    /// start can all ask for it at once.
    mic_apply: Mutex<()>,
    /// Which application was in front lately: where a saved clip's game comes from.
    activity: Arc<Mutex<ActivityLog>>,
    /// `settings.library.detectGame`, shared with the tracker thread.
    detect_game: Arc<AtomicBool>,
    /// Samples the foreground application while the buffer runs.
    tracker: Mutex<Option<ActivityTracker>>,
}

/// AAC encoder for the microphone lane (best effort, like the game audio).
fn aac_encoder(config: AudioConfig) -> Result<Box<dyn AudioEncoder>, String> {
    #[cfg(target_os = "macos")]
    {
        clipforge_core::audio::aac_audiotoolbox::AudioToolboxAacEncoder::new(config)
            .map(|e| Box::new(e) as Box<dyn AudioEncoder>)
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "windows")]
    {
        clipforge_core::audio::aac_mediafoundation::MediaFoundationAacEncoder::new(config)
            .map(|e| Box::new(e) as Box<dyn AudioEncoder>)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = config;
        Err("este sistema operativo todavía no tiene codificador de audio".into())
    }
}

impl Recorder {
    pub fn new(clips_dir: PathBuf, settings: Arc<SettingsStore>) -> Self {
        let engine = CaptureEngine::new(BufferConfig::default());
        let clips = Arc::new(ClipService::new(engine.buffer(), clips_dir));
        let current = settings.get();
        let mic = current.microphone;
        Self {
            engine: Mutex::new(engine),
            clips,
            last_start_error: Mutex::new(None),
            settings,
            mic_controls: MicControls::new(gain_from_percent(mic.gain_percent), mic.muted),
            mic: Mutex::new(MicRuntime::default()),
            mic_apply: Mutex::new(()),
            activity: Arc::new(Mutex::new(ActivityLog::new())),
            detect_game: Arc::new(AtomicBool::new(current.library.detect_game)),
            tracker: Mutex::new(None),
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
            .map_err(|e| e.to_string())?;
        self.start_activity_tracker();
        Ok(())
    }

    /// Start noticing which application is in front; a new buffer is a new
    /// timeline, so what was seen before is forgotten. Best effort: a source
    /// that can't be had only means clips carry no game.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn start_activity_tracker(&self) {
        let mut slot = self.tracker.lock().expect("tracker lock");
        *slot = None; // stop a previous one before starting over
        self.activity.lock().expect("activity lock").clear();
        *slot = system_source().map(|source| {
            ActivityTracker::start(
                source,
                Arc::clone(&self.activity),
                Arc::clone(&self.detect_game),
                SAMPLE_INTERVAL,
            )
        });
    }

    /// The game of a clip of the last `seconds`: the application that was in
    /// front for most of that time (never ClipForge itself or the desktop).
    /// `None` when detection is off or nothing stood out.
    pub fn detected_game(&self, seconds: u64) -> Option<String> {
        if !self.detect_game.load(Ordering::Relaxed) {
            return None;
        }
        let window = Duration::from_secs(seconds);
        self.activity
            .lock()
            .expect("activity lock")
            // To name a clip an application must hold the screen for at least
            // a fifth of it: a glance at something else doesn't count.
            .dominant(Instant::now(), window, window / 5, default_ignore)
            .map(|app| app.name)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn start(&self) -> Result<(), String> {
        Err("la captura en este sistema operativo llega con F-2.4 (Windows)".into())
    }

    /// Stop capturing; the buffered window survives so a clip can still be
    /// saved right after stopping.
    pub fn stop(&self) {
        // The engine detaches the microphone itself; forget our side of it.
        self.engine.lock().expect("engine lock").stop();
        // Stop looking; what was seen stays, so a clip saved right after
        // stopping still gets its game.
        *self.tracker.lock().expect("tracker lock") = None;
        *self.mic.lock().expect("mic lock") = MicRuntime::default();
        self.record_start_result(&Ok(())); // a deliberate stop isn't an error
    }

    // ---- microphone ----------------------------------------------------

    /// Bring the microphone in line with the current settings. Volume and
    /// mute take effect live; a new device or mode restarts only the
    /// microphone stream, never the video. May block for a while on the
    /// first microphone-permission prompt, so don't call it from the UI
    /// thread.
    pub fn apply_microphone(&self) -> MicStatus {
        let _serial = self.mic_apply.lock().expect("mic apply lock");
        let wanted = self.settings.get().microphone;
        self.apply_live_controls();

        let attached = self.mic.lock().expect("mic lock").applied.clone();
        match plan(&wanted, attached.as_ref(), self.is_running()) {
            MicAction::Nothing => {}
            MicAction::Detach => self.detach_microphone(),
            MicAction::Attach => {
                self.detach_microphone();
                self.attach_microphone(&wanted);
            }
        }
        self.mic_status()
    }

    /// Volume and mute only: atomics, instant, safe from any thread.
    pub fn apply_live_controls(&self) {
        let settings = self.settings.get();
        let wanted = settings.microphone;
        self.mic_controls
            .set_gain(gain_from_percent(wanted.gain_percent));
        self.mic_controls.set_muted(wanted.muted);
        self.detect_game
            .store(settings.library.detect_game, Ordering::Relaxed);
    }

    fn detach_microphone(&self) {
        self.engine.lock().expect("engine lock").detach_microphone();
        let mut mic = self.mic.lock().expect("mic lock");
        *mic = MicRuntime::default();
    }

    fn attach_microphone(&self, wanted: &MicrophoneSettings) {
        let result = self.try_attach_microphone(wanted);
        let mut mic = self.mic.lock().expect("mic lock");
        match result {
            Ok(()) => {
                mic.applied = Some(AppliedMic::from_settings(wanted));
                mic.attach_error = None;
                mic.since = Some(Instant::now());
            }
            Err(e) => {
                eprintln!("clipforge: microphone unavailable: {e}");
                mic.applied = None;
                mic.attach_error = Some(e);
                mic.since = None;
            }
        }
    }

    fn try_attach_microphone(&self, wanted: &MicrophoneSettings) -> Result<(), String> {
        let epoch = self
            .engine
            .lock()
            .expect("engine lock")
            .epoch()
            .ok_or("la captura todavía no empezó")?;
        let mode = match wanted.mode {
            MicMode::Mixed => CoreMicMode::Mixed,
            MicMode::Separate => CoreMicMode::Separate,
        };
        let encoder = match mode {
            CoreMicMode::Separate => Some(aac_encoder(mic_audio_config())?),
            CoreMicMode::Mixed => None,
        };
        let mut source = CpalMicSource::new(
            wanted.device_id.clone(),
            Arc::clone(&self.mic_controls),
            epoch,
        );
        // Opening the device can block on the permission prompt: do it
        // before taking the engine lock so the rest of the app stays live.
        let rx = source.start().map_err(|e| e.to_string())?;
        self.engine
            .lock()
            .expect("engine lock")
            .attach_microphone(Box::new(source), rx, mode, encoder)
            .map_err(|e| e.to_string())
    }

    pub fn mic_status(&self) -> MicStatus {
        let wanted = self.settings.get().microphone;
        let engine_attached = self
            .engine
            .lock()
            .expect("engine lock")
            .microphone_mode()
            .is_some();
        let mic = self.mic.lock().expect("mic lock");
        mic_status_from(MicFacts {
            enabled: wanted.enabled,
            muted: wanted.muted,
            attached: engine_attached && mic.applied.is_some(),
            attach_error: mic.attach_error.clone(),
            stream_error: self.mic_controls.error(),
            attached_for: mic.since.map(|t| t.elapsed()),
            heard_signal: self.mic_controls.input.heard_signal(),
            supported: cfg!(any(target_os = "macos", target_os = "windows")),
        })
    }

    /// Peak input level since the last call (for the Settings meter).
    pub fn take_mic_peak(&self) -> f32 {
        self.mic_controls.level.take_peak()
    }

    /// Microphones the user can choose from.
    pub fn list_microphones(&self) -> Result<Vec<MicDevice>, String> {
        list_input_devices()
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|d| MicDevice {
                        id: d.id,
                        name: d.name,
                        is_default: d.is_default,
                    })
                    .collect()
            })
            .map_err(|e| e.to_string())
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
    use crate::settings::Settings;

    fn recorder() -> Recorder {
        recorder_with(Settings::default())
    }

    fn recorder_with(settings: Settings) -> Recorder {
        let dir = std::env::temp_dir().join(format!(
            "clipforge-recorder-test-{}",
            crate::testutil::unique()
        ));
        let store = Arc::new(SettingsStore::load(dir.join("settings.json")));
        store.replace(settings).unwrap();
        Recorder::new(dir.join("clips"), store)
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

    fn mic_settings(f: impl FnOnce(&mut MicrophoneSettings)) -> Settings {
        let mut s = Settings::default();
        f(&mut s.microphone);
        s
    }

    #[test]
    fn a_fresh_recorder_reports_the_microphone_off() {
        let status = recorder().mic_status();
        assert!(!status.enabled && !status.active && !status.muted);
        assert_eq!(status.error, None);
    }

    #[test]
    fn enabling_without_a_running_buffer_waits_quietly() {
        // No capture, no clock: the microphone starts with the buffer.
        let recorder = recorder_with(mic_settings(|m| m.enabled = true));
        let status = recorder.apply_microphone();
        assert!(status.enabled && !status.active);
        assert_eq!(status.error, None, "waiting isn't an error");
    }

    #[test]
    fn volume_and_mute_apply_live_from_the_settings() {
        let recorder = recorder_with(mic_settings(|m| {
            m.gain_percent = 150;
            m.muted = true;
        }));
        let status = recorder.apply_microphone();
        assert!(status.muted);
        assert_eq!(recorder.mic_controls.gain(), 1.5);
        assert!(recorder.mic_controls.muted());

        recorder
            .settings
            .replace(mic_settings(|m| m.gain_percent = 30))
            .unwrap();
        recorder.apply_microphone();
        assert_eq!(recorder.mic_controls.gain(), 0.3);
        assert!(!recorder.mic_controls.muted());
    }

    /// Record that these applications were in front, each one starting this
    /// many seconds ago (oldest first).
    fn saw(recorder: &Recorder, history: &[(u64, &str)]) {
        use clipforge_core::activity::AppInfo;
        let now = Instant::now();
        let mut log = recorder.activity.lock().unwrap();
        for (ago, id) in history {
            log.record(
                now - Duration::from_secs(*ago),
                Some(AppInfo {
                    id: id.to_lowercase(),
                    name: id.to_string(),
                }),
            );
        }
    }

    #[test]
    fn a_clip_is_named_after_the_application_that_held_the_screen() {
        let recorder = recorder();
        saw(&recorder, &[(28, "Valorant"), (4, "Discord")]);
        assert_eq!(recorder.detected_game(30).as_deref(), Some("Valorant"));
        // A shorter clip is only about the last few seconds.
        assert_eq!(recorder.detected_game(3).as_deref(), Some("Discord"));
    }

    #[test]
    fn clipforge_itself_is_never_the_game() {
        let recorder = recorder();
        saw(&recorder, &[(28, "Valorant"), (15, "com.clipforge.app")]);
        // 13 s of the game and 15 s of ClipForge: only the game counts.
        assert_eq!(recorder.detected_game(30).as_deref(), Some("Valorant"));

        let only_ours = self::recorder();
        saw(&only_ours, &[(28, "com.clipforge.app")]);
        assert_eq!(only_ours.detected_game(30), None);
    }

    #[test]
    fn a_quick_glance_elsewhere_does_not_name_the_clip() {
        let recorder = recorder();
        saw(&recorder, &[(30, "Game"), (2, "Browser")]);
        assert_eq!(recorder.detected_game(30).as_deref(), Some("Game"));
        let glance_only = self::recorder();
        saw(&glance_only, &[(2, "Browser")]);
        assert_eq!(glance_only.detected_game(30), None);
    }

    #[test]
    fn nothing_seen_means_no_game() {
        assert_eq!(recorder().detected_game(30), None);
    }

    #[test]
    fn game_detection_follows_the_setting_live() {
        let mut off = Settings::default();
        off.library.detect_game = false;
        let recorder = recorder_with(off);
        saw(&recorder, &[(20, "Game")]);
        assert_eq!(recorder.detected_game(30), None, "off from the start");

        let mut on = Settings::default();
        on.library.detect_game = true;
        recorder.settings.replace(on).unwrap();
        recorder.apply_live_controls();
        assert_eq!(recorder.detected_game(30).as_deref(), Some("Game"));

        let mut off = Settings::default();
        off.library.detect_game = false;
        recorder.settings.replace(off).unwrap();
        recorder.apply_live_controls();
        assert_eq!(recorder.detected_game(30), None);
    }

    #[test]
    fn stopping_forgets_the_microphone_state() {
        let recorder = recorder_with(mic_settings(|m| m.enabled = true));
        recorder.mic.lock().unwrap().attach_error = Some("viejo".into());
        recorder.stop();
        assert_eq!(recorder.mic_status().error, None);
    }

    #[test]
    fn the_meter_reads_the_peak_once() {
        let recorder = recorder();
        recorder.mic_controls.level.record(&[0.4, -0.7]);
        assert_eq!(recorder.take_mic_peak(), 0.7);
        assert_eq!(recorder.take_mic_peak(), 0.0);
    }
}
