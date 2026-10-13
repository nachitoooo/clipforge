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
use clipforge_core::buffer::{BufferConfig, ReplayBuffer};
use clipforge_core::capture::AudioSource;
use clipforge_core::clip::{ClipService, SavedClip};
use clipforge_core::engine::CaptureEngine;
use serde::Serialize;

use crate::memory::{self, MemoryPlan};
use crate::mic::{
    gain_from_percent, plan, status as mic_status_from, AppliedMic, MicAction, MicDevice, MicFacts,
    MicStatus,
};
use crate::settings::{MicMode, MicrophoneSettings, SettingsStore};

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
    /// The engine's replay buffer, kept here so resizing it never waits for the
    /// engine lock (held while a capture starts).
    buffer: Arc<Mutex<ReplayBuffer>>,
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
            buffer: engine.buffer(),
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

    /// How much RAM the buffer may take under the current settings: the mode's
    /// bitrate and a length that covers the longest clip, no more.
    pub fn memory_plan(&self) -> MemoryPlan {
        let settings = self.settings.get();
        memory::plan(
            settings.performance.low_ram,
            settings.longest_clip_seconds(),
        )
    }

    /// Size the replay buffer for the current settings. Called as the capture
    /// starts (the bitrate is fixed for the whole run) and again whenever a clip
    /// length changes: the length is only an eviction window, so a shorter one
    /// gives memory back at once and a longer one fills up as time goes by.
    fn apply_memory_plan(&self) -> MemoryPlan {
        let plan = self.memory_plan();
        self.buffer
            .lock()
            .expect("replay buffer lock")
            .set_config(plan.buffer_config());
        plan
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

        let plan = self.apply_memory_plan();
        let config = EncoderConfig {
            bitrate: plan.bitrate,
            ..EncoderConfig::default() // documented 1080p60 defaults
        };
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

    /// Start the replay buffer on Windows: Graphics Capture of the primary
    /// monitor, WASAPI loopback for the sound, and the Media Foundation
    /// encoders for both.
    #[cfg(target_os = "windows")]
    pub fn start(&self) -> Result<(), String> {
        use clipforge_core::audio::aac_mediafoundation::MediaFoundationAacEncoder;
        use clipforge_core::audio::wasapi::WasapiLoopbackSource;
        use clipforge_core::audio::{AudioConfig, AudioEncoder};
        use clipforge_core::capture::clock::SharedEpoch;
        use clipforge_core::capture::combined::WithAudio;
        use clipforge_core::capture::scale::fit_even;
        use clipforge_core::capture::windows::{
            primary_display_size, WindowsCaptureConfig, WindowsGraphicsCaptureSource,
        };
        use clipforge_core::capture::FrameSource;
        use clipforge_core::encode::mediafoundation::MediaFoundationEncoder;
        use clipforge_core::encode::EncoderConfig;

        // The screen's own shape, shrunk to fit 1080p: a laptop's 1366x768 stays
        // as it is, a 4K monitor is recorded at 1920x1080.
        let (screen_w, screen_h) = primary_display_size().map_err(|e| e.to_string())?;
        let defaults = EncoderConfig::default();
        let (width, height) = fit_even(screen_w, screen_h, defaults.width, defaults.height);
        let plan = self.apply_memory_plan();
        let config = EncoderConfig {
            width,
            height,
            bitrate: plan.bitrate,
            ..defaults
        };

        // Sound is best effort: without an AAC encoder the clips are silent
        // instead of the whole buffer refusing to start.
        let audio_encoder: Option<Box<dyn AudioEncoder>> =
            match MediaFoundationAacEncoder::new(AudioConfig::default()) {
                Ok(encoder) => Some(Box::new(encoder)),
                Err(e) => {
                    eprintln!("clipforge: AAC encoder unavailable, clips will be silent: {e}");
                    None
                }
            };

        let epoch = SharedEpoch::new();
        let video = WindowsGraphicsCaptureSource::new(
            WindowsCaptureConfig {
                width,
                height,
                fps: config.fps,
                show_cursor: true,
            },
            epoch.clone(),
        );
        let source: Box<dyn FrameSource> = if audio_encoder.is_some() {
            Box::new(WithAudio::new(
                video,
                WasapiLoopbackSource::new(AudioConfig::default(), epoch),
            ))
        } else {
            Box::new(video)
        };
        let encoder = MediaFoundationEncoder::new(config).map_err(|e| e.to_string())?;
        self.engine
            .lock()
            .expect("engine lock")
            .start_with_audio(
                source,
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
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
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

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    pub fn start(&self) -> Result<(), String> {
        Err("la grabación todavía no está disponible en este sistema operativo".into())
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

    /// Volume, mute and the buffer's length: instant, safe from any thread.
    pub fn apply_live_controls(&self) {
        self.apply_memory_plan();
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

    // ---- low RAM mode ----

    fn low_ram_settings() -> Settings {
        let mut s = Settings::default();
        s.performance.low_ram = true;
        s
    }

    #[test]
    fn the_memory_plan_follows_the_setting() {
        assert_eq!(
            recorder().memory_plan(),
            memory::plan(false, crate::settings::DEFAULT_CLIP_SECONDS)
        );
        assert_eq!(
            recorder_with(low_ram_settings()).memory_plan(),
            memory::plan(true, crate::settings::DEFAULT_CLIP_SECONDS)
        );
    }

    fn buffer_config(recorder: &Recorder) -> clipforge_core::buffer::BufferConfig {
        let config = recorder.buffer.lock().unwrap().config().clone();
        config
    }

    #[test]
    fn starting_sizes_the_replay_buffer_for_the_clips_and_the_mode() {
        // The usual 30 s clip: 34 s of buffer, not the ceiling of two minutes.
        let normal = recorder();
        let plan = normal.apply_memory_plan();
        assert_eq!(plan.buffer_seconds, 30 + memory::MARGIN_SECONDS);
        assert_eq!(
            buffer_config(&normal).max_duration,
            Duration::from_secs(plan.buffer_seconds)
        );
        assert_eq!(buffer_config(&normal).max_bytes, plan.buffer_max_bytes);

        let low = recorder_with(low_ram_settings());
        let low_plan = low.apply_memory_plan();
        assert_eq!(low_plan.bitrate, memory::LOW_RAM.bitrate);
        assert_eq!(
            low_plan.buffer_seconds, plan.buffer_seconds,
            "same clips, same length"
        );
        assert!(buffer_config(&low).max_bytes < buffer_config(&normal).max_bytes);
    }

    #[test]
    fn low_ram_still_stops_a_long_clip_at_its_ceiling() {
        let mut settings = low_ram_settings();
        settings.shortcuts.clip_seconds = 120;
        let low = recorder_with(settings);
        let plan = low.apply_memory_plan();
        assert_eq!(plan, memory::LOW_RAM);
        let config = buffer_config(&low);
        assert_eq!(config.max_duration, Duration::from_secs(60));
        assert_eq!(config.max_bytes, 96 * 1024 * 1024);
    }

    #[test]
    fn turning_the_mode_off_again_gives_the_full_bitrate_back() {
        let recorder = recorder_with(low_ram_settings());
        recorder.apply_memory_plan();
        let mut back = Settings::default();
        back.performance.low_ram = false;
        recorder.settings.replace(back).unwrap();
        let plan = recorder.apply_memory_plan();
        assert_eq!(plan.bitrate, memory::NORMAL.bitrate);
        assert_eq!(
            buffer_config(&recorder).max_duration,
            Duration::from_secs(plan.buffer_seconds)
        );
    }

    #[test]
    fn changing_a_clip_length_resizes_the_buffer_without_a_restart() {
        let recorder = recorder();
        recorder.apply_live_controls();
        assert_eq!(
            buffer_config(&recorder).max_duration,
            Duration::from_secs(34)
        );

        let mut longer = Settings::default();
        longer.shortcuts.clip_seconds = 90;
        recorder.settings.replace(longer.clone()).unwrap();
        recorder.apply_live_controls();
        assert_eq!(
            buffer_config(&recorder).max_duration,
            Duration::from_secs(94)
        );

        // A second shortcut asking for more than the main one counts too.
        longer.shortcuts.clip_seconds = 20;
        longer.shortcuts.extra_save_clip = vec![crate::settings::ClipShortcut {
            accelerator: "F9".into(),
            seconds: 60,
        }];
        recorder.settings.replace(longer).unwrap();
        recorder.apply_live_controls();
        assert_eq!(
            buffer_config(&recorder).max_duration,
            Duration::from_secs(64)
        );
    }
}
