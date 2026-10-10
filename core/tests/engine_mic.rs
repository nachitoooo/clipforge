//! The microphone in the engine (platform independent, mock encoders):
//! attached and detached while the capture runs, as a separate lane/track or
//! mixed into the game audio.
//!
//! Sources are realtime synthetic ones stamped on a shared clock (`base`), the
//! same way the real microphone shares the video epoch.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clipforge_core::audio::mock::MockAacEncoder;
use clipforge_core::audio::test_tone::{TestToneConfig, TestToneSource};
use clipforge_core::audio::{AudioConfig, AudioEncoder, MicMode};
use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::combined::WithAudio;
use clipforge_core::capture::test_pattern::{TestPatternConfig, TestPatternSource};
use clipforge_core::capture::AudioSource;
use clipforge_core::clip::ClipService;
use clipforge_core::encode::mock::MockEncoder;
use clipforge_core::encode::EncoderConfig;
use clipforge_core::engine::{CaptureEngine, EngineError};

fn video_config() -> EncoderConfig {
    EncoderConfig {
        width: 64,
        height: 48,
        fps: 60,
        keyframe_interval: 30,
        ..Default::default()
    }
}

fn mic_config() -> AudioConfig {
    AudioConfig {
        sample_rate: 48_000,
        channels: 1,
        bitrate: 96_000,
    }
}

struct Session {
    engine: CaptureEngine,
    clips: Arc<ClipService>,
    base: Instant,
    dir: std::path::PathBuf,
}

impl Session {
    /// A running capture: realtime video + realtime game audio.
    fn start(name: &str, with_audio: bool) -> Self {
        let dir = std::env::temp_dir().join("clipforge-engine-mic").join(name);
        std::fs::remove_dir_all(&dir).ok();
        let base = Instant::now();
        let mut engine = CaptureEngine::new(BufferConfig::default());
        let clips = Arc::new(ClipService::new(engine.buffer(), &dir));
        let video = TestPatternSource::new(TestPatternConfig {
            width: 64,
            height: 48,
            fps: 60,
            realtime: true,
            ..Default::default()
        });
        let source: Box<dyn clipforge_core::capture::FrameSource> = if with_audio {
            Box::new(WithAudio::new(
                video,
                TestToneSource::new(TestToneConfig {
                    realtime: true,
                    clock_base: Some(base),
                    ..Default::default()
                }),
            ))
        } else {
            Box::new(video)
        };
        let audio: Option<Box<dyn AudioEncoder>> = with_audio.then(|| {
            Box::new(MockAacEncoder::new(AudioConfig::default())) as Box<dyn AudioEncoder>
        });
        engine
            .start_with_audio(
                source,
                Box::new(MockEncoder::new(video_config())),
                audio,
                Arc::clone(&clips),
            )
            .expect("engine starts");
        Self {
            engine,
            clips,
            base,
            dir,
        }
    }

    /// Start a mono 880 Hz "microphone" on the shared clock and attach it.
    fn attach(&mut self, mode: MicMode) -> Result<(), EngineError> {
        let mut mic = TestToneSource::new(TestToneConfig {
            channels: 1,
            frequency: 880.0,
            realtime: true,
            clock_base: Some(self.base),
            ..Default::default()
        });
        let rx = mic.start().expect("mic source starts");
        let encoder: Option<Box<dyn AudioEncoder>> = (mode == MicMode::Separate)
            .then(|| Box::new(MockAacEncoder::new(mic_config())) as Box<dyn AudioEncoder>);
        self.engine
            .attach_microphone(Box::new(mic), rx, mode, encoder)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.engine.stop();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn audio_tracks(path: &Path) -> usize {
    let size = std::fs::metadata(path).unwrap().len();
    let mp4 = mp4::Mp4Reader::read_header(BufReader::new(File::open(path).unwrap()), size).unwrap();
    mp4.tracks()
        .values()
        .filter(|t| t.track_type().unwrap() == mp4::TrackType::Audio)
        .count()
}

fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[test]
fn a_separate_microphone_becomes_a_second_audio_track() {
    let mut s = Session::start("separate", true);
    pause(400);
    s.attach(MicMode::Separate).unwrap();
    assert_eq!(s.engine.microphone_mode(), Some(MicMode::Separate));
    pause(1800);

    let saved = s.clips.save_last(Duration::from_secs(1)).unwrap();
    assert!(
        saved.audio_packets > 20,
        "game audio: {}",
        saved.audio_packets
    );
    assert!(saved.mic_packets > 20, "mic audio: {}", saved.mic_packets);
    assert_eq!(audio_tracks(&saved.path), 2);
    assert!(s.engine.stats().mic_packets_buffered > 30);
    assert_eq!(s.engine.stats().mic_errors, 0);
}

#[test]
fn a_mixed_microphone_keeps_one_audio_track() {
    let mut s = Session::start("mixed", true);
    pause(400);
    s.attach(MicMode::Mixed).unwrap();
    assert_eq!(s.engine.microphone_mode(), Some(MicMode::Mixed));
    pause(1500);

    let saved = s.clips.save_last(Duration::from_secs(1)).unwrap();
    assert!(saved.audio_packets > 20);
    assert_eq!(saved.mic_packets, 0, "the voice is inside the main track");
    assert_eq!(audio_tracks(&saved.path), 1);
}

#[test]
fn detaching_takes_the_track_out_of_new_clips() {
    let mut s = Session::start("detach", true);
    pause(300);
    s.attach(MicMode::Separate).unwrap();
    pause(1500);
    assert_eq!(
        audio_tracks(&s.clips.save_last(Duration::from_secs(1)).unwrap().path),
        2
    );

    s.engine.detach_microphone();
    assert_eq!(s.engine.microphone_mode(), None);
    pause(300);
    let after = s.clips.save_last(Duration::from_secs(1)).unwrap();
    assert_eq!(after.mic_packets, 0);
    assert_eq!(audio_tracks(&after.path), 1);

    s.engine.detach_microphone(); // idempotent
}

#[test]
fn reattaching_starts_a_fresh_lane_without_a_hole() {
    let mut s = Session::start("reattach", true);
    pause(300);
    s.attach(MicMode::Separate).unwrap();
    pause(600);
    s.engine.detach_microphone();
    pause(500); // the microphone is off for a while...
    s.attach(MicMode::Separate).unwrap();
    pause(1500);

    // ...and the clip only carries the new, contiguous stretch.
    let saved = s.clips.save_last(Duration::from_secs(2)).unwrap();
    assert!(saved.mic_packets > 20);
    assert!(
        saved.mic_packets < 95,
        "old stretch must be gone: {}",
        saved.mic_packets
    );
    assert_eq!(audio_tracks(&saved.path), 2);
}

#[test]
fn switching_modes_live_works_in_both_directions() {
    let mut s = Session::start("switch", true);
    pause(300);
    s.attach(MicMode::Mixed).unwrap();
    pause(500);
    s.engine.detach_microphone();
    s.attach(MicMode::Separate).unwrap();
    pause(1200);
    let separate = s.clips.save_last(Duration::from_secs(1)).unwrap();
    assert_eq!(audio_tracks(&separate.path), 2);

    s.engine.detach_microphone();
    s.attach(MicMode::Mixed).unwrap();
    pause(600);
    let mixed = s.clips.save_last(Duration::from_secs(1)).unwrap();
    assert_eq!(audio_tracks(&mixed.path), 1);
}

#[test]
fn attaching_is_refused_when_it_cannot_work() {
    // Not running.
    let mut idle = CaptureEngine::new(BufferConfig::default());
    let mut mic = TestToneSource::new(TestToneConfig {
        channels: 1,
        realtime: true,
        ..Default::default()
    });
    let rx = mic.start().unwrap();
    assert!(matches!(
        idle.attach_microphone(Box::new(mic), rx, MicMode::Mixed, None),
        Err(EngineError::NotRunning)
    ));

    // Already attached.
    let mut s = Session::start("refused", true);
    s.attach(MicMode::Mixed).unwrap();
    assert!(matches!(
        s.attach(MicMode::Mixed),
        Err(EngineError::MicrophoneAttached)
    ));
    s.engine.detach_microphone();

    // A separate lane needs an encoder.
    let mut mic = TestToneSource::new(TestToneConfig {
        channels: 1,
        realtime: true,
        clock_base: Some(s.base),
        ..Default::default()
    });
    let rx = mic.start().unwrap();
    assert!(matches!(
        s.engine
            .attach_microphone(Box::new(mic), rx, MicMode::Separate, None),
        Err(EngineError::AudioUnavailable)
    ));
}

#[test]
fn mixing_needs_game_audio_to_mix_into() {
    let mut s = Session::start("no-audio", false);
    assert!(matches!(
        s.attach(MicMode::Mixed),
        Err(EngineError::AudioUnavailable)
    ));
    // A separate microphone doesn't depend on the game audio.
    s.attach(MicMode::Separate).unwrap();
    pause(1500);
    let saved = s.clips.save_last(Duration::from_secs(1)).unwrap();
    assert_eq!(saved.audio_packets, 0);
    assert!(saved.mic_packets > 20);
}

#[test]
fn stopping_with_a_microphone_attached_does_not_hang() {
    for mode in [MicMode::Mixed, MicMode::Separate] {
        let mut s = Session::start("stop", true);
        pause(200);
        s.attach(mode).unwrap();
        pause(300);
        s.engine.stop();
        assert!(!s.engine.is_running());
        assert_eq!(s.engine.microphone_mode(), None);
    }
}

#[test]
fn a_microphone_that_ends_by_itself_does_not_break_the_capture() {
    let mut s = Session::start("mic-ends", true);
    pause(200);
    let mut mic = TestToneSource::new(TestToneConfig {
        channels: 1,
        realtime: true,
        clock_base: Some(s.base),
        duration_limit: Some(Duration::from_millis(300)),
        ..Default::default()
    });
    let rx = mic.start().unwrap();
    s.engine
        .attach_microphone(Box::new(mic), rx, MicMode::Mixed, None)
        .unwrap();
    pause(1200);
    assert!(s.engine.is_running());
    let saved = s.clips.save_last(Duration::from_secs(1)).unwrap();
    assert!(saved.audio_packets > 20);
}
