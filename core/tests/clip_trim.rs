//! Trimming saved clips: structure of the result (frames, keyframes, edit
//! lists, tracks), that the video bytes are copied untouched, that the audio
//! stays aligned to the picture, and that a clip can be trimmed again.
//!
//! Clips are synthetic: every video frame and audio packet carries its own
//! index in its payload, so the tests can tell *which* frame plays first
//! without decoding anything. Decoding (the waveform) is tested against a
//! small real clip made by ffmpeg, `tests/fixtures/tone.mp4`.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clipforge_core::audio::AudioPacket;
use clipforge_core::clip::mp4mux::{write_mp4_with_tracks, AudioParams, VideoParams};
use clipforge_core::clip::trim::{
    audio_peaks, probe, trim_clip, TrimError, TrimSpec, PEAKS_PER_SECOND,
};
use clipforge_core::encode::{Codec, EncodedPacket, ParameterSets};

const FPS: u64 = 60;
const GOP: u64 = 120; // 2 s, like the real encoder
const RATE: u64 = 48_000;
const PRIMING: u32 = 1024;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir()
            .join("clipforge-clip-trim")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn frame_time(n: u64) -> Duration {
    // Rounded up so the muxer's truncation gives exactly n * 1500 ticks.
    Duration::from_nanos((n * 1_000_000_000).div_ceil(FPS))
}

/// A frame whose payload says which frame it is.
fn video_packets(count: u64) -> Vec<EncodedPacket> {
    (0..count)
        .map(|n| {
            let mut data = vec![0, 0, 0, 20];
            data.extend_from_slice(&(n as u32).to_be_bytes());
            data.extend_from_slice(&[if n % GOP == 0 { 0x65 } else { 0x41 }; 16]);
            EncodedPacket {
                data,
                pts: frame_time(n),
                dts: frame_time(n),
                is_keyframe: n % GOP == 0,
            }
        })
        .collect()
}

/// Contiguous AAC packets whose payload says which packet they are.
fn audio_packets(count: u64) -> Vec<AudioPacket> {
    (0..count)
        .map(|k| {
            let mut data = (k as u16).to_be_bytes().to_vec();
            data.extend_from_slice(&[0xAA; 200]);
            AudioPacket {
                data,
                pts: Duration::from_nanos(k * 1024 * 1_000_000_000 / RATE),
                duration: Duration::from_nanos(1024 * 1_000_000_000 / RATE),
            }
        })
        .collect()
}

fn audio_params(channels: u16) -> AudioParams {
    AudioParams {
        sample_rate: RATE as u32,
        channels,
        bitrate: 128_000,
        priming_frames: PRIMING,
    }
}

fn video_params() -> (VideoParams, ParameterSets) {
    (
        VideoParams {
            codec: Codec::H264,
            width: 64,
            height: 48,
            fps: FPS as u32,
        },
        ParameterSets {
            sps: vec![vec![0x67, 0x42, 0xc0, 0x0d, 0x96, 0x54, 0x0a, 0x0f, 0xc8]],
            pps: vec![vec![0x68, 0xce, 0x3c, 0x80]],
        },
    )
}

/// A 6 s clip with a game-audio track (stereo) and a voice track (mono).
fn make_source(dir: &Scratch, name: &str) -> PathBuf {
    let path = dir.path(name);
    let (video, sets) = video_params();
    let game = audio_packets(6 * RATE / 1024 + 4);
    let voice = audio_packets(6 * RATE / 1024 + 4);
    write_mp4_with_tracks(
        &path,
        &video,
        &sets,
        &video_packets(6 * FPS),
        &[
            (&audio_params(2), game.as_slice()),
            (&audio_params(1), voice.as_slice()),
        ],
    )
    .unwrap();
    path
}

fn spec(start: f64, end: f64, keep_audio: &[usize]) -> TrimSpec {
    TrimSpec {
        start: Duration::from_secs_f64(start),
        end: Duration::from_secs_f64(end),
        keep_audio: keep_audio.to_vec(),
    }
}

// ---- reading results back ---------------------------------------------------

struct Reader {
    mp4: mp4::Mp4Reader<BufReader<File>>,
}

impl Reader {
    fn open(path: &Path) -> Self {
        let size = std::fs::metadata(path).unwrap().len();
        let mp4 =
            mp4::Mp4Reader::read_header(BufReader::new(File::open(path).unwrap()), size).unwrap();
        Self { mp4 }
    }

    fn track_ids(&self, kind: mp4::TrackType) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .mp4
            .tracks()
            .iter()
            .filter(|(_, t)| t.track_type().unwrap() == kind)
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn video_id(&self) -> u32 {
        self.track_ids(mp4::TrackType::Video)[0]
    }

    fn audio_tracks(&self) -> usize {
        self.track_ids(mp4::TrackType::Audio).len()
    }

    /// `(delay seconds, skipped media ticks)` of a track's edit list.
    fn edit(&self, id: u32) -> (f64, i64) {
        let track = &self.mp4.tracks()[&id];
        let entries = track
            .trak
            .edts
            .as_ref()
            .and_then(|e| e.elst.as_ref())
            .map(|e| e.entries.clone())
            .unwrap_or_default();
        let empty = |m: u64| m == u64::from(u32::MAX) || m == u64::MAX;
        let movie = f64::from(self.mp4.timescale());
        let delay: f64 = entries
            .iter()
            .filter(|e| empty(e.media_time))
            .map(|e| e.segment_duration as f64 / movie)
            .sum();
        let skip = entries
            .iter()
            .find(|e| !empty(e.media_time))
            .map_or(0, |e| e.media_time as i64);
        (delay, skip)
    }

    fn video_sample_count(&self) -> u32 {
        self.mp4.tracks()[&self.video_id()].sample_count()
    }

    /// Which original frame is the n-th stored sample (from its payload).
    fn frame_at(&mut self, sample: u32) -> u32 {
        let id = self.video_id();
        let s = self.mp4.read_sample(id, sample + 1).unwrap().unwrap();
        u32::from_be_bytes(s.bytes[4..8].try_into().unwrap())
    }

    fn is_sync(&mut self, sample: u32) -> bool {
        let id = self.video_id();
        self.mp4
            .read_sample(id, sample + 1)
            .unwrap()
            .unwrap()
            .is_sync
    }

    /// `(packet number, playback time in seconds)` of every packet of the
    /// `index`-th audio track.
    fn audio_times(&mut self, index: usize) -> Vec<(u16, f64)> {
        let id = self.track_ids(mp4::TrackType::Audio)[index];
        let (delay, skip) = self.edit(id);
        let count = self.mp4.tracks()[&id].sample_count();
        (1..=count)
            .map(|s| {
                let sample = self.mp4.read_sample(id, s).unwrap().unwrap();
                let at = (sample.start_time as i64 - skip) as f64 / RATE as f64 + delay;
                (u16::from_be_bytes([sample.bytes[0], sample.bytes[1]]), at)
            })
            .collect()
    }
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

// ---- probe ------------------------------------------------------------------

#[test]
fn probe_reports_what_the_editor_needs() {
    let dir = Scratch::new("probe");
    let source = make_source(&dir, "source.mp4");
    let info = probe(&source).unwrap();

    assert!(near(info.duration.as_secs_f64(), 6.0, 0.02), "{info:?}");
    assert_eq!((info.width, info.height), (64, 48));
    assert_eq!(info.frames, 360);
    assert!(near(info.fps, 60.0, 0.01), "fps {}", info.fps);
    assert_eq!(info.audio.len(), 2);
    assert_eq!(
        (info.audio[0].index, info.audio[0].channels),
        (0, 2),
        "the default track first"
    );
    assert_eq!((info.audio[1].index, info.audio[1].channels), (1, 1));
    assert!(near(info.audio[0].duration.as_secs_f64(), 6.0, 0.1));
}

#[test]
fn a_silent_clip_has_no_audio_lanes() {
    let dir = Scratch::new("silent");
    let path = dir.path("silent.mp4");
    let (video, sets) = video_params();
    write_mp4_with_tracks(&path, &video, &sets, &video_packets(120), &[]).unwrap();
    assert!(probe(&path).unwrap().audio.is_empty());
}

// ---- cutting ----------------------------------------------------------------

#[test]
fn a_cut_starting_between_keyframes_is_frame_accurate_through_an_edit_list() {
    let dir = Scratch::new("smart-cut");
    let source = make_source(&dir, "source.mp4");
    let out = dir.path("cut.mp4");

    let result = trim_clip(&source, &out, &spec(2.5, 4.5, &[0, 1])).unwrap();
    assert!(near(result.duration.as_secs_f64(), 2.0, 0.02), "{result:?}");
    assert_eq!(result.video_frames, 120);
    assert_eq!(result.audio_tracks, 2);

    let mut cut = Reader::open(&out);
    // Decoding starts at the keyframe at 2.0 s (frame 120): 30 frames of
    // hidden lead-in, then the 120 shown ones.
    assert_eq!(cut.video_sample_count(), 150);
    assert_eq!(cut.frame_at(0), 120);
    assert!(cut.is_sync(0), "the file must start on a keyframe");
    let (_, skip) = cut.edit(cut.video_id());
    assert_eq!(skip, 30 * 1500, "playback starts at frame 150");
    assert_eq!(cut.frame_at(30), 150, "frame 150 is the first one shown");
    assert_eq!(cut.frame_at(149), 269);

    // What players report is the visible length, not the stored one.
    let info = probe(&out).unwrap();
    assert!(near(info.duration.as_secs_f64(), 2.0, 0.02), "{info:?}");
    assert_eq!(info.frames, 120);
    assert!(
        near(cut.mp4.duration().as_secs_f64(), 2.0, 0.02),
        "container duration {:?}",
        cut.mp4.duration()
    );
}

#[test]
fn a_cut_on_a_keyframe_needs_no_video_edit_list() {
    let dir = Scratch::new("on-keyframe");
    let source = make_source(&dir, "source.mp4");
    let out = dir.path("cut.mp4");

    trim_clip(&source, &out, &spec(2.0, 4.0, &[0])).unwrap();

    let mut cut = Reader::open(&out);
    assert_eq!(cut.video_sample_count(), 120);
    assert_eq!(cut.frame_at(0), 120);
    assert_eq!(cut.edit(cut.video_id()), (0.0, 0));
    assert!(near(probe(&out).unwrap().duration.as_secs_f64(), 2.0, 0.02));
}

#[test]
fn video_bytes_are_copied_untouched() {
    let dir = Scratch::new("copy");
    let source = make_source(&dir, "source.mp4");
    let out = dir.path("cut.mp4");
    trim_clip(&source, &out, &spec(1.0, 3.0, &[])).unwrap();

    let mut original = Reader::open(&source);
    let mut cut = Reader::open(&out);
    let (original_id, cut_id) = (original.video_id(), cut.video_id());
    for sample in 0..cut.video_sample_count() {
        let copy = cut.mp4.read_sample(cut_id, sample + 1).unwrap().unwrap();
        let frame = u32::from_be_bytes(copy.bytes[4..8].try_into().unwrap());
        let source_sample = original
            .mp4
            .read_sample(original_id, frame + 1)
            .unwrap()
            .unwrap();
        assert_eq!(copy.bytes, source_sample.bytes, "frame {frame}");
        assert_eq!(copy.is_sync, source_sample.is_sync, "frame {frame}");
    }
}

#[test]
fn the_original_is_left_exactly_as_it_was() {
    let dir = Scratch::new("original");
    let source = make_source(&dir, "source.mp4");
    let before = std::fs::read(&source).unwrap();
    trim_clip(&source, &dir.path("cut.mp4"), &spec(1.0, 2.0, &[0, 1])).unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), before);
}

#[test]
fn the_start_and_end_of_the_clip_are_valid_bounds() {
    let dir = Scratch::new("bounds");
    let source = make_source(&dir, "source.mp4");

    let whole = dir.path("whole.mp4");
    let result = trim_clip(&source, &whole, &spec(0.0, 100.0, &[0, 1])).unwrap();
    assert_eq!(
        result.video_frames, 360,
        "an end past the clip means 'to the end'"
    );
    assert_eq!(Reader::open(&whole).video_sample_count(), 360);

    let tail = dir.path("tail.mp4");
    trim_clip(&source, &tail, &spec(5.5, 6.0, &[0])).unwrap();
    let info = probe(&tail).unwrap();
    assert!(near(info.duration.as_secs_f64(), 0.5, 0.03), "{info:?}");
}

#[test]
fn a_selection_shorter_than_a_frame_keeps_one_frame() {
    let dir = Scratch::new("tiny");
    let source = make_source(&dir, "source.mp4");
    let out = dir.path("tiny.mp4");
    let result = trim_clip(&source, &out, &spec(1.0, 1.004, &[])).unwrap();
    assert_eq!(result.video_frames, 1);
    assert_eq!(probe(&out).unwrap().frames, 1);
}

#[test]
fn a_clip_can_be_trimmed_again() {
    let dir = Scratch::new("again");
    let source = make_source(&dir, "source.mp4");
    let first = dir.path("first.mp4");
    let second = dir.path("second.mp4");
    trim_clip(&source, &first, &spec(2.5, 4.5, &[0, 1])).unwrap();
    // 0.5 s into the first cut = original frame 180, still ahead of its only
    // early keyframe (original frame 120, hidden lead-in included).
    trim_clip(&first, &second, &spec(0.5, 1.5, &[0, 1])).unwrap();

    let mut cut = Reader::open(&second);
    assert_eq!(
        cut.frame_at(0),
        120,
        "decoding restarts from the same keyframe"
    );
    let (_, skip) = cut.edit(cut.video_id());
    assert_eq!(skip, 60 * 1500);
    assert_eq!(cut.frame_at(60), 180);
    let info = probe(&second).unwrap();
    assert!(near(info.duration.as_secs_f64(), 1.0, 0.02), "{info:?}");
    assert_eq!(info.audio.len(), 2);
}

// ---- audio ------------------------------------------------------------------

#[test]
fn audio_stays_aligned_to_the_picture() {
    let dir = Scratch::new("sync");
    let source = make_source(&dir, "source.mp4");
    let out = dir.path("cut.mp4");
    let start = 2.5;
    trim_clip(&source, &out, &spec(start, 4.5, &[0, 1])).unwrap();

    let mut original = Reader::open(&source);
    let mut cut = Reader::open(&out);
    for track in 0..2 {
        let before = original.audio_times(track);
        let after = cut.audio_times(track);
        assert!(!after.is_empty());
        let mut compared = 0;
        for (packet, time) in &after {
            let (_, original_time) = before.iter().find(|(p, _)| p == packet).unwrap();
            // Every packet plays exactly `start` earlier than it did.
            assert!(
                near(*time, original_time - start, 0.000_05),
                "track {track} packet {packet}: {time} vs {original_time} - {start}"
            );
            compared += 1;
        }
        assert!(compared > 90, "{compared} packets kept");
        // Only the neighbourhood of the cut is kept, not the whole track.
        assert!(
            after.len() < 130,
            "track {track} kept {} packets",
            after.len()
        );
        // The first sound that plays is the one that played at `start`.
        let playing = after
            .iter()
            .filter(|(_, t)| *t <= 0.0)
            .map(|(p, _)| *p)
            .max()
            .unwrap();
        let expected = (start * RATE as f64 / 1024.0) as u16;
        assert!(
            playing.abs_diff(expected) <= 2,
            "playing {playing}, expected about {expected}"
        );
    }
}

#[test]
fn audio_stays_aligned_after_a_second_cut() {
    let dir = Scratch::new("sync-twice");
    let source = make_source(&dir, "source.mp4");
    let first = dir.path("first.mp4");
    let second = dir.path("second.mp4");
    trim_clip(&source, &first, &spec(2.5, 5.0, &[0])).unwrap();
    trim_clip(&first, &second, &spec(0.7, 2.0, &[0])).unwrap();

    let mut original = Reader::open(&source);
    let mut cut = Reader::open(&second);
    let before = original.audio_times(0);
    for (packet, time) in cut.audio_times(0) {
        let (_, original_time) = before.iter().find(|(p, _)| *p == packet).unwrap();
        assert!(
            near(time, original_time - 2.5 - 0.7, 0.000_1),
            "packet {packet}: {time} vs {original_time}"
        );
    }
}

#[test]
fn dropping_a_track_leaves_the_others_in_order() {
    let dir = Scratch::new("tracks");
    let source = make_source(&dir, "source.mp4");

    let game_only = dir.path("game.mp4");
    let kept = trim_clip(&source, &game_only, &spec(1.0, 3.0, &[0])).unwrap();
    assert_eq!(kept.audio_tracks, 1);
    let info = probe(&game_only).unwrap();
    assert_eq!(info.audio.len(), 1);
    assert_eq!(info.audio[0].channels, 2, "the stereo game track");

    let voice_only = dir.path("voice.mp4");
    trim_clip(&source, &voice_only, &spec(1.0, 3.0, &[1])).unwrap();
    let info = probe(&voice_only).unwrap();
    assert_eq!(info.audio.len(), 1);
    assert_eq!(info.audio[0].channels, 1, "the mono voice track");

    let silent = dir.path("silent.mp4");
    let none = trim_clip(&source, &silent, &spec(1.0, 3.0, &[])).unwrap();
    assert_eq!(none.audio_tracks, 0);
    assert_eq!(Reader::open(&silent).audio_tracks(), 0);

    // Asking twice for a track, or in the wrong order, still keeps file order.
    let both = dir.path("both.mp4");
    trim_clip(&source, &both, &spec(1.0, 3.0, &[1, 0, 1])).unwrap();
    let info = probe(&both).unwrap();
    assert_eq!(info.audio.len(), 2);
    assert_eq!(info.audio[0].channels, 2);
}

// ---- errors -----------------------------------------------------------------

#[test]
fn bad_requests_fail_cleanly_and_leave_no_file() {
    let dir = Scratch::new("errors");
    let source = make_source(&dir, "source.mp4");
    let out = dir.path("never.mp4");

    let empty = trim_clip(&source, &out, &spec(3.0, 3.0, &[]));
    assert!(matches!(empty, Err(TrimError::EmptySelection)), "{empty:?}");
    let backwards = trim_clip(&source, &out, &spec(4.0, 3.0, &[]));
    assert!(matches!(backwards, Err(TrimError::EmptySelection)));
    let beyond = trim_clip(&source, &out, &spec(10.0, 12.0, &[]));
    assert!(matches!(beyond, Err(TrimError::EmptySelection)));
    let missing_track = trim_clip(&source, &out, &spec(1.0, 2.0, &[5]));
    assert!(matches!(missing_track, Err(TrimError::NoSuchAudioTrack(5))));
    assert!(!out.exists(), "a failed trim leaves nothing behind");

    let missing = trim_clip(&dir.path("nope.mp4"), &out, &spec(0.0, 1.0, &[]));
    assert!(matches!(missing, Err(TrimError::Io(_))));
}

#[test]
fn files_that_are_not_clips_are_refused() {
    let dir = Scratch::new("garbage");
    let junk = dir.path("junk.mp4");
    std::fs::write(&junk, b"this is definitely not an mp4 file").unwrap();
    assert!(probe(&junk).is_err());
    assert!(trim_clip(&junk, &dir.path("out.mp4"), &spec(0.0, 1.0, &[])).is_err());
    assert!(audio_peaks(&junk, 0).is_err());
}

// ---- real AAC (made by ffmpeg) ---------------------------------------------

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.mp4")
}

/// Mean level of `values[from..to]` (0–255).
fn mean(values: &[u8], from: usize, to: usize) -> f64 {
    let slice = &values[from..to];
    slice.iter().map(|&v| f64::from(v)).sum::<f64>() / slice.len() as f64
}

#[test]
fn the_waveform_shows_when_the_sound_is() {
    // 4 s clip: silence for 2 s, then a 440 Hz tone at half scale.
    let peaks = audio_peaks(&fixture(), 0).unwrap();
    assert_eq!(peaks.per_second, PEAKS_PER_SECOND);
    let pps = PEAKS_PER_SECOND as usize;
    assert!(
        near(peaks.values.len() as f64, 4.0 * pps as f64, 2.0),
        "{} buckets",
        peaks.values.len()
    );
    let quiet = mean(&peaks.values, 2, 2 * pps - 6);
    let loud = mean(&peaks.values, 2 * pps + 6, 4 * pps - 2);
    assert!(quiet < 6.0, "silence reads {quiet}");
    assert!(
        (100.0..160.0).contains(&loud),
        "a 0.5 amplitude tone reads about 128, got {loud}"
    );
}

#[test]
fn a_real_clip_from_another_encoder_can_be_cut_and_its_sound_stays_in_place() {
    let dir = Scratch::new("tone");
    let out = dir.path("cut.mp4");
    // Keyframes are 1 s apart in the fixture, so 1.5 s is mid-GOP. The tone
    // starts at 2.0 s, i.e. 0.5 s into the cut.
    let result = trim_clip(&fixture(), &out, &spec(1.5, 3.5, &[0])).unwrap();
    assert!(near(result.duration.as_secs_f64(), 2.0, 0.05), "{result:?}");

    let info = probe(&out).unwrap();
    assert!(near(info.duration.as_secs_f64(), 2.0, 0.05), "{info:?}");
    assert_eq!(info.audio.len(), 1);

    let pps = PEAKS_PER_SECOND as usize;
    let peaks = audio_peaks(&out, 0).unwrap();
    let quiet = mean(&peaks.values, 2, pps / 2 - 6);
    let loud = mean(&peaks.values, pps / 2 + 6, 2 * pps - 6);
    assert!(quiet < 6.0, "before the tone reads {quiet}");
    assert!(loud > 100.0, "the tone reads {loud}");
}

#[test]
fn the_fixture_clip_probes_as_a_normal_clip() {
    let info = probe(&fixture()).unwrap();
    assert!(near(info.duration.as_secs_f64(), 4.0, 0.05), "{info:?}");
    assert_eq!((info.width, info.height), (64, 48));
    assert_eq!(info.frames, 120);
    assert_eq!(info.audio.len(), 1);
    assert_eq!(info.audio[0].channels, 1);
}
