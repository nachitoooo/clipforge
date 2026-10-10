//! F-2.5 end-to-end (platform independent): video + audio through the
//! engine into the buffer, and out as an .mp4 with two tracks.
//! TestPatternSource + TestToneSource → mock encoders → ReplayBuffer →
//! ClipService → mp4.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use clipforge_core::audio::mock::MockAacEncoder;
use clipforge_core::audio::test_tone::{TestToneConfig, TestToneSource};
use clipforge_core::audio::{AudioChunk, AudioConfig};
use clipforge_core::buffer::BufferConfig;
use clipforge_core::capture::combined::WithAudio;
use clipforge_core::capture::test_pattern::{TestPatternConfig, TestPatternSource};
use clipforge_core::capture::{AudioSource, CaptureError};
use clipforge_core::clip::ClipService;
use clipforge_core::encode::mock::MockEncoder;
use clipforge_core::encode::EncoderConfig;
use clipforge_core::engine::CaptureEngine;
use tokio::sync::mpsc;

fn video_config() -> EncoderConfig {
    EncoderConfig {
        width: 64,
        height: 48,
        fps: 60,
        keyframe_interval: 30,
        ..Default::default()
    }
}

fn video_source(seconds: u64) -> TestPatternSource {
    let config = video_config();
    TestPatternSource::new(TestPatternConfig {
        width: config.width,
        height: config.height,
        fps: config.fps,
        frame_limit: Some(seconds * 60),
        realtime: false,
    })
}

fn tone(seconds: u64) -> TestToneSource {
    TestToneSource::new(TestToneConfig {
        duration_limit: Some(Duration::from_secs(seconds)),
        ..Default::default()
    })
}

fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("clipforge-engine-audio")
        .join(name);
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn read_mp4(path: &Path) -> mp4::Mp4Reader<BufReader<File>> {
    let size = std::fs::metadata(path).unwrap().len();
    mp4::Mp4Reader::read_header(BufReader::new(File::open(path).unwrap()), size).unwrap()
}

fn track_of(mp4: &mp4::Mp4Reader<BufReader<File>>, kind: mp4::TrackType) -> Option<&mp4::Mp4Track> {
    mp4.tracks()
        .values()
        .find(|t| t.track_type().unwrap() == kind)
}

fn run_engine<S: clipforge_core::capture::FrameSource + 'static>(
    source: S,
    dir: &Path,
) -> (CaptureEngine, Arc<ClipService>) {
    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(engine.buffer(), dir));
    engine
        .start_with_audio(
            Box::new(source),
            Box::new(MockEncoder::new(video_config())),
            Some(Box::new(MockAacEncoder::new(AudioConfig::default()))),
            Arc::clone(&clips),
        )
        .expect("engine starts");
    (engine, clips)
}

#[test]
fn clips_carry_an_aac_track_in_sync_with_the_video() {
    let dir = test_dir("sync");
    let (mut engine, clips) = run_engine(WithAudio::new(video_source(10), tone(10)), &dir);
    engine.join();

    let stats = engine.stats();
    assert_eq!(stats.packets_buffered, 600);
    assert!(stats.audio_packets_buffered >= 468, "got {stats:?}");
    assert_eq!(stats.audio_errors, 0);

    let saved = clips.save_last(Duration::from_secs(5)).unwrap();
    assert!(
        saved.audio_packets > 200,
        "audio packets: {}",
        saved.audio_packets
    );

    let mp4 = read_mp4(&saved.path);
    assert_eq!(mp4.tracks().len(), 2);
    let video = track_of(&mp4, mp4::TrackType::Video).unwrap();
    let audio = track_of(&mp4, mp4::TrackType::Audio).unwrap();
    assert_eq!(audio.timescale(), 48_000);
    let (v, a) = (video.duration(), audio.duration());
    assert!(
        v.abs_diff(a) < Duration::from_millis(50),
        "video {v:?} and audio {a:?} must cover the same span"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_source_without_audio_yields_video_only_clips() {
    let dir = test_dir("silent");
    let (mut engine, clips) = run_engine(video_source(5), &dir); // no take_audio
    engine.join();

    assert_eq!(engine.stats().audio_packets_buffered, 0);
    let saved = clips.save_last(Duration::from_secs(3)).unwrap();
    assert_eq!(saved.audio_packets, 0);
    assert_eq!(read_mp4(&saved.path).tracks().len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn engine_without_an_audio_encoder_ignores_the_audio_stream() {
    let dir = test_dir("no-encoder");
    let mut engine = CaptureEngine::new(BufferConfig::default());
    let clips = Arc::new(ClipService::new(engine.buffer(), &dir));
    engine
        .start(
            Box::new(WithAudio::new(video_source(3), tone(3))),
            Box::new(MockEncoder::new(video_config())),
            Arc::clone(&clips),
        )
        .unwrap();
    engine.join();
    let saved = clips.save_last(Duration::from_secs(2)).unwrap();
    assert_eq!(saved.audio_packets, 0);
    assert_eq!(read_mp4(&saved.path).tracks().len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// Plays back pre-built chunks, so a test can script holes and overlaps.
struct ScriptedAudio(Option<Vec<AudioChunk>>);

impl AudioSource for ScriptedAudio {
    fn start(&mut self) -> Result<mpsc::Receiver<AudioChunk>, CaptureError> {
        let chunks = self.0.take().ok_or(CaptureError::AlreadyStarted)?;
        let (tx, rx) = mpsc::channel(chunks.len().max(1));
        for chunk in chunks {
            tx.try_send(chunk).unwrap();
        }
        Ok(rx) // tx dropped here: the stream ends after the script
    }
    fn stop(&mut self) {}
}

fn chunk_at(ms: u64) -> AudioChunk {
    AudioChunk {
        samples: vec![0.25; 480 * 2], // 10 ms stereo
        channels: 2,
        sample_rate: 48_000,
        timestamp: Duration::from_millis(ms),
    }
}

#[test]
fn a_hole_in_the_audio_is_filled_so_later_audio_does_not_drift() {
    let dir = test_dir("gap");
    // 4 s of audio with a 500 ms hole between 1.0 s and 1.5 s.
    let mut chunks: Vec<_> = (0..100).map(|i| chunk_at(i * 10)).collect();
    chunks.extend((150..400).map(|i| chunk_at(i * 10)));
    let source = WithAudio::new(video_source(4), ScriptedAudio(Some(chunks)));
    let (mut engine, clips) = run_engine(source, &dir);
    engine.join();

    let saved = clips.save_last(Duration::from_secs(4)).unwrap();
    let mp4 = read_mp4(&saved.path);
    let video = track_of(&mp4, mp4::TrackType::Video).unwrap().duration();
    let audio = track_of(&mp4, mp4::TrackType::Audio).unwrap().duration();
    // Without the silence fill the audio would be 500 ms short.
    assert!(
        video.abs_diff(audio) < Duration::from_millis(60),
        "video {video:?} audio {audio:?}"
    );
    assert_eq!(engine.stats().audio_errors, 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stopping_mid_stream_joins_both_workers() {
    let dir = test_dir("stop");
    let realtime_tone = TestToneSource::new(TestToneConfig {
        realtime: true,
        ..Default::default()
    });
    let video = TestPatternSource::new(TestPatternConfig {
        width: 64,
        height: 48,
        realtime: true,
        ..Default::default()
    });
    let (mut engine, _clips) = run_engine(WithAudio::new(video, realtime_tone), &dir);
    std::thread::sleep(Duration::from_millis(300));
    assert!(engine.is_running());
    engine.stop(); // must not hang
    assert!(!engine.is_running());
    assert!(engine.stats().audio_packets_buffered > 0);
    engine.stop(); // idempotent
    std::fs::remove_dir_all(&dir).ok();
}
