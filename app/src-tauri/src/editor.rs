//! Clip editor backend: describe a clip, draw its audio, cut a piece out of it.
//!
//! The heavy lifting is `clipforge_core::clip::trim`; this module is the thin,
//! Tauri-free layer in between. It validates what the webview sends (the UI
//! only ever names clips by file name, like in `library.rs`), turns core types
//! into plain JSON the UI can use, and picks the name of the trimmed copy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clipforge_core::clip::trim::{self, TrimSpec};
use serde::{Deserialize, Serialize};

use crate::library;

/// Suffix of a trimmed copy: `clip-20261009-101500.mp4` →
/// `clip-20261009-101500-recorte.mp4`.
const TRIM_SUFFIX: &str = "-recorte";

/// What the editor shows about a clip.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorClip {
    pub name: String,
    /// Seconds.
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub frames: usize,
    /// Empty for a silent clip: the editor then shows no audio lanes.
    pub audio: Vec<EditorAudioTrack>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorAudioTrack {
    /// Position in the file; what `keepAudio` refers to.
    pub index: usize,
    pub channels: u16,
    pub sample_rate: u32,
    pub duration: f64,
}

/// Loudness over time of one audio track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    pub per_second: u32,
    /// 0–255 per bucket.
    pub peaks: Vec<u8>,
}

/// A cut asked for by the editor.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimRequest {
    pub name: String,
    /// Seconds from the start of the clip.
    pub start: f64,
    pub end: f64,
    /// Audio tracks to keep; the rest are dropped from the copy.
    pub keep_audio: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimResult {
    /// File name of the new clip.
    pub name: String,
    pub duration: f64,
    pub bytes: u64,
}

pub fn describe(dir: &Path, name: &str) -> Result<EditorClip, String> {
    let path = library::clip_path(dir, name)?;
    let info = trim::probe(&path).map_err(|e| e.to_string())?;
    Ok(EditorClip {
        name: name.to_string(),
        duration: info.duration.as_secs_f64(),
        width: info.width,
        height: info.height,
        fps: info.fps,
        frames: info.frames,
        audio: info
            .audio
            .into_iter()
            .map(|a| EditorAudioTrack {
                index: a.index,
                channels: a.channels,
                sample_rate: a.sample_rate,
                duration: a.duration.as_secs_f64(),
            })
            .collect(),
    })
}

pub fn waveform(dir: &Path, name: &str, track: usize) -> Result<Waveform, String> {
    let path = library::clip_path(dir, name)?;
    let peaks = trim::audio_peaks(&path, track).map_err(|e| e.to_string())?;
    Ok(Waveform {
        per_second: peaks.per_second,
        peaks: peaks.values,
    })
}

/// Check a request from the webview and turn it into a core [`TrimSpec`].
fn spec_of(request: &TrimRequest) -> Result<TrimSpec, String> {
    let seconds = |value: f64, what: &str| {
        if value.is_finite() && (0.0..=86_400.0).contains(&value) {
            Ok(Duration::from_secs_f64(value))
        } else {
            Err(format!("{what} no válido: {value}"))
        }
    };
    let start = seconds(request.start, "el inicio")?;
    let end = seconds(request.end, "el final")?;
    if end <= start {
        return Err("el final tiene que ser posterior al inicio".into());
    }
    Ok(TrimSpec {
        start,
        end,
        keep_audio: request.keep_audio.clone(),
    })
}

/// Where the trimmed copy of `name` goes: `<stem>-recorte.mp4`, then
/// `-recorte-2`, `-recorte-3`… An already trimmed clip trims into the same
/// family instead of piling up suffixes.
pub fn trimmed_path(dir: &Path, name: &str) -> Result<PathBuf, String> {
    library::clip_path(dir, name)?; // validates the name
    let stem = name
        .strip_suffix(".mp4")
        .or_else(|| name.strip_suffix(".MP4"))
        .unwrap_or(name);
    let base = family_base(stem);
    let mut path = dir.join(format!("{base}{TRIM_SUFFIX}.mp4"));
    let mut n = 2u32;
    while path.exists() {
        path = dir.join(format!("{base}{TRIM_SUFFIX}-{n}.mp4"));
        n += 1;
    }
    Ok(path)
}

/// The name a family of trimmed copies hangs from: `clip-recorte-2` → `clip`.
/// Only a suffix at the very end counts; `mi-recorte-final` stays as it is.
fn family_base(stem: &str) -> &str {
    let Some(at) = stem.rfind(TRIM_SUFFIX) else {
        return stem;
    };
    let rest = &stem[at + TRIM_SUFFIX.len()..];
    let is_copy = rest.is_empty()
        || rest
            .strip_prefix('-')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if is_copy {
        &stem[..at]
    } else {
        stem
    }
}

pub fn trim_clip(dir: &Path, request: &TrimRequest) -> Result<TrimResult, String> {
    let spec = spec_of(request)?;
    let source = library::clip_path(dir, &request.name)?;
    let target = trimmed_path(dir, &request.name)?;
    let done = trim::trim_clip(&source, &target, &spec).map_err(|e| e.to_string())?;
    Ok(TrimResult {
        name: target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        duration: done.duration.as_secs_f64(),
        bytes: done.bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Scratch clips folder holding a copy of the 4 s test clip
    /// (`core/tests/fixtures/tone.mp4`: one keyframe per second, silence for
    /// 2 s and then a tone).
    struct Folder(PathBuf);

    impl Folder {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "clipforge-editor-{label}-{}",
                crate::testutil::unique()
            ));
            fs::create_dir_all(&dir).unwrap();
            let fixture =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../core/tests/fixtures/tone.mp4");
            fs::copy(fixture, dir.join("clip.mp4")).unwrap();
            Self(dir)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).ok();
        }
    }

    fn request(name: &str, start: f64, end: f64, keep_audio: &[usize]) -> TrimRequest {
        TrimRequest {
            name: name.into(),
            start,
            end,
            keep_audio: keep_audio.to_vec(),
        }
    }

    #[test]
    fn describing_a_clip_lists_its_audio_lanes() {
        let dir = Folder::new("describe");
        let clip = describe(&dir.0, "clip.mp4").unwrap();
        assert_eq!(clip.name, "clip.mp4");
        assert!((clip.duration - 4.0).abs() < 0.05, "{clip:?}");
        assert_eq!((clip.width, clip.height), (64, 48));
        assert_eq!(clip.frames, 120);
        assert_eq!(clip.audio.len(), 1);
        assert_eq!(clip.audio[0].index, 0);
        assert_eq!(clip.audio[0].channels, 1);
    }

    #[test]
    fn the_json_the_ui_receives_is_camel_case() {
        let dir = Folder::new("json");
        let json = serde_json::to_value(describe(&dir.0, "clip.mp4").unwrap()).unwrap();
        assert!(json["audio"][0]["sampleRate"].is_number());
        assert!(json["fps"].is_number());
        let wave = serde_json::to_value(waveform(&dir.0, "clip.mp4", 0).unwrap()).unwrap();
        assert_eq!(wave["perSecond"], 50);
        assert!(wave["peaks"].as_array().unwrap().len() >= 190);
    }

    #[test]
    fn requests_arrive_in_camel_case() {
        let parsed: TrimRequest =
            serde_json::from_str(r#"{"name":"a.mp4","start":1.5,"end":3,"keepAudio":[0,2]}"#)
                .unwrap();
        assert_eq!(parsed, request("a.mp4", 1.5, 3.0, &[0, 2]));
    }

    #[test]
    fn the_waveform_of_a_missing_track_is_an_error() {
        let dir = Folder::new("no-track");
        assert!(waveform(&dir.0, "clip.mp4", 3).is_err());
    }

    #[test]
    fn trimming_writes_a_copy_and_keeps_the_original() {
        let dir = Folder::new("trim");
        let before = fs::read(dir.0.join("clip.mp4")).unwrap();
        let done = trim_clip(&dir.0, &request("clip.mp4", 1.5, 3.5, &[0])).unwrap();

        assert_eq!(done.name, "clip-recorte.mp4");
        assert!((done.duration - 2.0).abs() < 0.05, "{done:?}");
        assert!(done.bytes > 0);
        assert_eq!(fs::read(dir.0.join("clip.mp4")).unwrap(), before);
        let copy = describe(&dir.0, &done.name).unwrap();
        assert!((copy.duration - 2.0).abs() < 0.05, "{copy:?}");
        assert_eq!(copy.audio.len(), 1);
    }

    #[test]
    fn dropping_the_audio_makes_a_silent_copy() {
        let dir = Folder::new("silent");
        let done = trim_clip(&dir.0, &request("clip.mp4", 0.0, 2.0, &[])).unwrap();
        assert!(describe(&dir.0, &done.name).unwrap().audio.is_empty());
    }

    #[test]
    fn copies_never_overwrite_each_other() {
        let dir = Folder::new("unique");
        let first = trim_clip(&dir.0, &request("clip.mp4", 0.0, 1.0, &[0])).unwrap();
        let second = trim_clip(&dir.0, &request("clip.mp4", 1.0, 2.0, &[0])).unwrap();
        let third = trim_clip(&dir.0, &request("clip.mp4", 2.0, 3.0, &[0])).unwrap();
        assert_eq!(first.name, "clip-recorte.mp4");
        assert_eq!(second.name, "clip-recorte-2.mp4");
        assert_eq!(third.name, "clip-recorte-3.mp4");
    }

    #[test]
    fn trimming_a_copy_stays_in_the_same_family_of_names() {
        let dir = Folder::new("family");
        let first = trim_clip(&dir.0, &request("clip.mp4", 0.0, 3.0, &[0])).unwrap();
        // "clip-recorte.mp4" exists, so its own trim is "clip-recorte-2.mp4",
        // not "clip-recorte-recorte.mp4".
        let again = trim_clip(&dir.0, &request(&first.name, 0.0, 1.0, &[0])).unwrap();
        assert_eq!(again.name, "clip-recorte-2.mp4");
        let more = trim_clip(&dir.0, &request("clip-recorte-2.mp4", 0.0, 0.5, &[0])).unwrap();
        assert_eq!(more.name, "clip-recorte-3.mp4");
    }

    #[test]
    fn names_that_merely_contain_the_suffix_are_kept() {
        let dir = Folder::new("lookalike");
        assert_eq!(
            trimmed_path(&dir.0, "mi-recorte-final.mp4")
                .unwrap()
                .file_name()
                .unwrap(),
            "mi-recorte-final-recorte.mp4"
        );
    }

    #[test]
    fn bad_requests_are_refused_before_touching_any_file() {
        let dir = Folder::new("bad");
        for (start, end) in [
            (2.0, 2.0),
            (3.0, 1.0),
            (-1.0, 1.0),
            (f64::NAN, 1.0),
            (0.0, f64::INFINITY),
            (0.0, 1e9),
        ] {
            let result = trim_clip(&dir.0, &request("clip.mp4", start, end, &[0]));
            assert!(result.is_err(), "({start}, {end}) should be refused");
        }
        // Only the original is in the folder.
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn names_cannot_escape_the_clips_folder() {
        let dir = Folder::new("escape");
        for name in [
            "../clip.mp4",
            "sub/clip.mp4",
            "..",
            "clip.txt",
            "",
            "captura.png",
        ] {
            assert!(describe(&dir.0, name).is_err(), "{name:?}");
            assert!(waveform(&dir.0, name, 0).is_err(), "{name:?}");
            assert!(
                trim_clip(&dir.0, &request(name, 0.0, 1.0, &[])).is_err(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn a_missing_clip_is_a_readable_error() {
        let dir = Folder::new("missing");
        let err = describe(&dir.0, "gone.mp4").unwrap_err();
        assert!(err.contains("clip"), "{err}");
    }
}
