//! Adding an `edts`/`elst` edit list to a finished .mp4.
//!
//! An AAC stream decodes with ~2 000 samples of encoder delay, and a clip cut
//! out of the middle of a stream also needs a block of decoder warm-up before
//! its first valid sample. The container's way to say "skip this much at the
//! start" is an edit list; every conforming player honors it. The `mp4` crate
//! can't write one, but its writer always appends `moov` *after* `mdat`, so
//! growing `moov` never shifts a chunk offset. We patch just the tail of the
//! file: find `moov`, insert `edts` into the audio `trak`, fix two sizes.
//!
//! A *trimmed* clip uses the same mechanism on the video track: the cut may
//! only start on a keyframe, so the frames between that keyframe and the
//! requested start stay in the file as decode-only lead-in and the video's edit
//! list skips them (a "smart cut" — no re-encoding, frame-accurate start).

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::mp4mux::MuxError;

/// One edit list entry, in the movie timescale (`segment_duration`) and the
/// track's media timescale (`media_time`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditEntry {
    pub segment_duration: u32,
    /// Start in the media, or `-1` for an empty edit (silence/offset).
    pub media_time: i32,
}

pub(crate) fn edts_box(entries: &[EditEntry]) -> Vec<u8> {
    let elst_len = 8 + 4 + 4 + 12 * entries.len();
    let mut out = Vec::with_capacity(8 + elst_len);
    out.extend_from_slice(&((8 + elst_len) as u32).to_be_bytes());
    out.extend_from_slice(b"edts");
    out.extend_from_slice(&(elst_len as u32).to_be_bytes());
    out.extend_from_slice(b"elst");
    out.extend_from_slice(&[0, 0, 0, 0]); // version 0, flags 0
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for e in entries {
        out.extend_from_slice(&e.segment_duration.to_be_bytes());
        out.extend_from_slice(&e.media_time.to_be_bytes());
        out.extend_from_slice(&1i16.to_be_bytes()); // media rate 1.0
        out.extend_from_slice(&0i16.to_be_bytes());
    }
    out
}

/// `(fourcc, start, end)` of every box in `buf`, which must hold a sequence
/// of whole boxes.
fn boxes(buf: &[u8]) -> Result<Vec<([u8; 4], usize, usize)>, MuxError> {
    let bad = || MuxError::Io(std::io::Error::other("malformed mp4 box structure"));
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let header = buf.get(pos..pos + 8).ok_or_else(bad)?;
        let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = header[4..8].try_into().unwrap();
        let end = match size {
            0 => buf.len(),
            1 => return Err(bad()), // 64-bit boxes don't occur inside moov
            n => pos
                .checked_add(n)
                .filter(|&e| e <= buf.len())
                .ok_or_else(bad)?,
        };
        if end < pos + 8 {
            return Err(bad());
        }
        out.push((kind, pos, end));
        pos = end;
    }
    Ok(out)
}

/// The `hdlr` handler type (`vide`, `soun`, ...) of a `trak` payload.
fn handler_type(trak_payload: &[u8]) -> Result<Option<[u8; 4]>, MuxError> {
    for (kind, start, end) in boxes(trak_payload)? {
        if &kind != b"mdia" {
            continue;
        }
        let mdia = &trak_payload[start + 8..end];
        for (kind, s, e) in boxes(mdia)? {
            // hdlr: 8 header + 4 version/flags + 4 pre_defined + 4 handler type
            if &kind == b"hdlr" && e - s >= 20 {
                return Ok(Some(mdia[s + 16..s + 20].try_into().expect("4 bytes")));
            }
        }
    }
    Ok(None)
}

/// Overwrite the duration of an `mvhd` or `tkhd` box (versions 0 and 1) in
/// place. Returns false when the box is too short to hold the field.
fn set_box_duration(b: &mut [u8], duration: u32) -> bool {
    let kind: [u8; 4] = match b.get(4..8).map(|k| k.try_into().expect("4 bytes")) {
        Some(k) => k,
        None => return false,
    };
    let Some(&version) = b.get(8) else {
        return false;
    };
    // header(8) + version/flags(4) + creation and modification times, then
    // the timescale (mvhd) or track id and a reserved word (tkhd).
    let times = if version == 1 { 16 } else { 8 };
    let offset = match &kind {
        b"mvhd" => 12 + times + 4,
        b"tkhd" => 12 + times + 8,
        _ => return false,
    };
    let width = if version == 1 { 8 } else { 4 };
    let Some(field) = b.get_mut(offset..offset + width) else {
        return false;
    };
    if width == 8 {
        field.copy_from_slice(&u64::from(duration).to_be_bytes());
    } else {
        field.copy_from_slice(&duration.to_be_bytes());
    }
    true
}

/// What [`patch_moov_with`] changes in a `moov` box.
#[derive(Default)]
pub(crate) struct MoovPatch<'a> {
    /// `edts` box for the video track.
    pub video_edts: Option<&'a [u8]>,
    /// `edts` for the i-th sound track (file order); `None` leaves it alone.
    pub sound_edts: &'a [Option<Vec<u8>>],
    /// Presentation length in the movie timescale, written into `mvhd` and the
    /// video `tkhd` (their sample sums still count any hidden lead-in).
    pub movie_duration: Option<u32>,
}

/// Insert an `edts` box after the `tkhd` of the sound tracks of a complete
/// `moov` box: `edts_per_sound_track[i]` goes to the i-th sound track (in
/// file order); `None` leaves that track alone. Returns the new `moov`
/// bytes, or `Ok(None)` if nothing needed patching.
#[cfg(test)]
pub(crate) fn patch_moov(
    moov: &[u8],
    edts_per_sound_track: &[Option<Vec<u8>>],
) -> Result<Option<Vec<u8>>, MuxError> {
    patch_moov_with(
        moov,
        &MoovPatch {
            sound_edts: edts_per_sound_track,
            ..Default::default()
        },
    )
}

/// The general form of [`patch_moov`]: edit lists for the video and sound
/// tracks and the presentation duration.
pub(crate) fn patch_moov_with(
    moov: &[u8],
    patch: &MoovPatch<'_>,
) -> Result<Option<Vec<u8>>, MuxError> {
    let bad = || MuxError::Io(std::io::Error::other("malformed moov box"));
    if moov.len() < 8 || &moov[4..8] != b"moov" {
        return Err(bad());
    }
    let payload = &moov[8..];
    let mut out = moov[..8].to_vec();
    let mut sound_index = 0usize;
    let mut patched_any = false;

    for (kind, start, end) in boxes(payload)? {
        let child = &payload[start..end];
        if &kind == b"mvhd" {
            let mut mvhd = child.to_vec();
            if let Some(duration) = patch.movie_duration {
                patched_any |= set_box_duration(&mut mvhd, duration);
            }
            out.extend_from_slice(&mvhd);
            continue;
        }
        if &kind != b"trak" {
            out.extend_from_slice(child);
            continue;
        }

        let handler = handler_type(&child[8..])?;
        let is_video = handler.as_ref() == Some(b"vide");
        let edts = match handler.as_ref() {
            Some(b"vide") => patch.video_edts,
            Some(b"soun") => {
                let edts = patch.sound_edts.get(sound_index).and_then(|e| e.as_deref());
                sound_index += 1;
                edts
            }
            _ => None,
        };
        let new_duration = patch.movie_duration.filter(|_| is_video);
        if edts.is_none() && new_duration.is_none() {
            out.extend_from_slice(child);
            continue;
        }

        let (tkhd_start, tkhd_end) = boxes(&child[8..])?
            .into_iter()
            .find(|(k, _, _)| k == b"tkhd")
            .map(|(_, s, e)| (8 + s, 8 + e))
            .ok_or_else(bad)?;
        let mut tkhd = child[tkhd_start..tkhd_end].to_vec();
        if let Some(duration) = new_duration {
            patched_any |= set_box_duration(&mut tkhd, duration);
        }
        let edts = edts.unwrap_or(&[]);
        let mut trak = Vec::with_capacity(child.len() + edts.len());
        trak.extend_from_slice(&child[..tkhd_start]);
        trak.extend_from_slice(&tkhd);
        trak.extend_from_slice(edts);
        trak.extend_from_slice(&child[tkhd_end..]);
        let trak_len = trak.len() as u32;
        trak[..4].copy_from_slice(&trak_len.to_be_bytes());
        out.extend_from_slice(&trak);
        patched_any |= !edts.is_empty();
    }
    if !patched_any {
        return Ok(None);
    }
    let out_len = out.len() as u32;
    out[..4].copy_from_slice(&out_len.to_be_bytes());
    Ok(Some(out))
}

/// Add `entries` as the edit list of the (single) sound track of the
/// finished file at `path`. See [`add_audio_edit_lists`].
pub fn add_audio_edit_list(path: &Path, entries: &[EditEntry]) -> Result<(), MuxError> {
    add_audio_edit_lists(path, &[entries.to_vec()])
}

/// Give each sound track of the finished file at `path` its edit list:
/// `per_sound_track[i]` is for the i-th sound track in file order (an empty
/// list leaves that track without one). The file must end with its `moov`
/// box (as the `mp4` writer leaves it).
pub fn add_audio_edit_lists(
    path: &Path,
    per_sound_track: &[Vec<EditEntry>],
) -> Result<(), MuxError> {
    add_edit_lists(
        path,
        &ClipEdits {
            audio: per_sound_track.to_vec(),
            ..Default::default()
        },
    )
}

/// Edit lists (and the presentation length) to write into a finished file.
#[derive(Debug, Clone, Default)]
pub struct ClipEdits {
    /// Edit list of the video track; empty leaves it without one.
    pub video: Vec<EditEntry>,
    /// One list per sound track in file order; empty leaves that track alone.
    pub audio: Vec<Vec<EditEntry>>,
    /// Presentation length in the movie timescale (see [`MoovPatch`]).
    pub movie_duration: Option<u32>,
}

/// Write `edits` into the finished file at `path`; the file must end with
/// its `moov` box (as the `mp4` writer leaves it).
pub fn add_edit_lists(path: &Path, edits: &ClipEdits) -> Result<(), MuxError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let len = file.metadata()?.len();

    // Walk the top-level boxes to find `moov`.
    let mut pos = 0u64;
    let mut moov_start = None;
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let size32 = u32::from_be_bytes(header[..4].try_into().unwrap());
        let size = match size32 {
            0 => len - pos,
            1 => {
                let mut large = [0u8; 8];
                file.read_exact(&mut large)?;
                u64::from_be_bytes(large)
            }
            n => u64::from(n),
        };
        if &header[4..8] == b"moov" {
            moov_start = Some(pos);
            break;
        }
        if size < 8 {
            break;
        }
        pos += size;
    }
    let moov_start = moov_start
        .ok_or_else(|| MuxError::Io(std::io::Error::other("no moov box in the finished file")))?;

    file.seek(SeekFrom::Start(moov_start))?;
    let mut moov = Vec::new();
    file.read_to_end(&mut moov)?;

    let video = (!edits.video.is_empty()).then(|| edts_box(&edits.video));
    let sound: Vec<Option<Vec<u8>>> = edits
        .audio
        .iter()
        .map(|entries| (!entries.is_empty()).then(|| edts_box(entries)))
        .collect();
    let patch = MoovPatch {
        video_edts: video.as_deref(),
        sound_edts: &sound,
        movie_duration: edits.movie_duration,
    };
    if let Some(patched) = patch_moov_with(&moov, &patch)? {
        file.set_len(moov_start)?;
        file.seek(SeekFrom::Start(moov_start))?;
        file.write_all(&patched)?;
        file.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(payload);
        b
    }

    fn hdlr(handler: &[u8; 4]) -> Vec<u8> {
        let mut p = vec![0u8; 4 + 4]; // version/flags + pre_defined
        p.extend_from_slice(handler);
        p.extend_from_slice(&[0u8; 12]);
        make_box(b"hdlr", &p)
    }

    fn trak(handler: &[u8; 4], marker: u8) -> Vec<u8> {
        let tkhd = make_box(b"tkhd", &[marker; 16]);
        let mdia = make_box(
            b"mdia",
            &[make_box(b"mdhd", &[0; 8]), hdlr(handler)].concat(),
        );
        make_box(b"trak", &[tkhd, mdia].concat())
    }

    fn moov(traks: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = make_box(b"mvhd", &[0; 20]);
        for t in traks {
            payload.extend_from_slice(t);
        }
        make_box(b"moov", &payload)
    }

    fn entries() -> Vec<EditEntry> {
        vec![
            EditEntry {
                segment_duration: 900,
                media_time: -1,
            },
            EditEntry {
                segment_duration: 90_000,
                media_time: 1088,
            },
        ]
    }

    #[test]
    fn edts_box_layout_is_spec_exact() {
        let edts = edts_box(&entries());
        assert_eq!(&edts[4..8], b"edts");
        assert_eq!(
            u32::from_be_bytes(edts[..4].try_into().unwrap()) as usize,
            edts.len()
        );
        assert_eq!(&edts[12..16], b"elst");
        assert_eq!(u32::from_be_bytes(edts[20..24].try_into().unwrap()), 2); // entry count
                                                                             // First entry: 900, -1, rate 1.0
        assert_eq!(u32::from_be_bytes(edts[24..28].try_into().unwrap()), 900);
        assert_eq!(i32::from_be_bytes(edts[28..32].try_into().unwrap()), -1);
        assert_eq!(i16::from_be_bytes(edts[32..34].try_into().unwrap()), 1);
        // Second entry's media_time
        assert_eq!(i32::from_be_bytes(edts[40..44].try_into().unwrap()), 1088);
        assert_eq!(edts.len(), 8 + 8 + 4 + 4 + 24);
    }

    #[test]
    fn inserts_edts_after_tkhd_of_the_sound_track_only() {
        let original = moov(&[trak(b"vide", 1), trak(b"soun", 2)]);
        let edts = edts_box(&entries());
        let patched = patch_moov(&original, &[Some(edts.clone())])
            .unwrap()
            .unwrap();

        assert_eq!(patched.len(), original.len() + edts.len());
        // Outer sizes were fixed up.
        assert_eq!(
            u32::from_be_bytes(patched[..4].try_into().unwrap()) as usize,
            patched.len()
        );

        // Re-parse: video trak untouched, audio trak = tkhd, edts, mdia.
        let payload = &patched[8..];
        let traks: Vec<_> = boxes(payload)
            .unwrap()
            .into_iter()
            .filter(|(k, _, _)| k == b"trak")
            .collect();
        assert_eq!(traks.len(), 2);
        let video_children: Vec<_> = boxes(&payload[traks[0].1 + 8..traks[0].2])
            .unwrap()
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        assert_eq!(video_children, [*b"tkhd", *b"mdia"]);
        let audio_children: Vec<_> = boxes(&payload[traks[1].1 + 8..traks[1].2])
            .unwrap()
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        assert_eq!(audio_children, [*b"tkhd", *b"edts", *b"mdia"]);
    }

    #[test]
    fn a_file_without_a_sound_track_is_left_alone() {
        let original = moov(&[trak(b"vide", 1)]);
        assert!(patch_moov(&original, &[Some(edts_box(&entries()))])
            .unwrap()
            .is_none());
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(patch_moov(b"nope", &[Some(edts_box(&entries()))]).is_err());
        let mut truncated = moov(&[trak(b"soun", 2)]);
        truncated.truncate(truncated.len() - 5);
        assert!(patch_moov(&truncated, &[Some(edts_box(&entries()))]).is_err());
    }

    #[test]
    fn patches_a_file_on_disk_in_place() {
        let dir = std::env::temp_dir().join("clipforge-edit-list-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake.mp4");
        let mdat = make_box(b"mdat", &[7u8; 1000]);
        let file = [
            make_box(b"ftyp", b"isom\0\0\0\0"),
            mdat.clone(),
            moov(&[trak(b"vide", 1), trak(b"soun", 2)]),
        ]
        .concat();
        std::fs::write(&path, &file).unwrap();

        add_audio_edit_list(&path, &entries()).unwrap();

        let patched = std::fs::read(&path).unwrap();
        assert_eq!(patched.len(), file.len() + edts_box(&entries()).len());
        // Everything before moov (ftyp + mdat) is byte-identical.
        let prefix = file.len() - moov(&[trak(b"vide", 1), trak(b"soun", 2)]).len();
        assert_eq!(&patched[..prefix], &file[..prefix]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn each_sound_track_gets_its_own_edit_list() {
        let original = moov(&[trak(b"vide", 1), trak(b"soun", 2), trak(b"soun", 3)]);
        let first = edts_box(&entries());
        let second = edts_box(&[EditEntry {
            segment_duration: 5,
            media_time: 7,
        }]);
        let patched = patch_moov(&original, &[Some(first.clone()), Some(second.clone())])
            .unwrap()
            .unwrap();
        assert_eq!(patched.len(), original.len() + first.len() + second.len());

        let payload = &patched[8..];
        let traks: Vec<_> = boxes(payload)
            .unwrap()
            .into_iter()
            .filter(|(k, _, _)| k == b"trak")
            .collect();
        let children = |i: usize| -> Vec<[u8; 4]> {
            boxes(&payload[traks[i].1 + 8..traks[i].2])
                .unwrap()
                .into_iter()
                .map(|(k, _, _)| k)
                .collect()
        };
        assert_eq!(children(0), [*b"tkhd", *b"mdia"]);
        assert_eq!(children(1), [*b"tkhd", *b"edts", *b"mdia"]);
        assert_eq!(children(2), [*b"tkhd", *b"edts", *b"mdia"]);
    }

    #[test]
    fn a_track_without_an_edit_list_is_skipped_but_still_counted() {
        // Two sound tracks, only the second needs an edit list.
        let original = moov(&[trak(b"vide", 1), trak(b"soun", 2), trak(b"soun", 3)]);
        let edts = edts_box(&entries());
        let patched = patch_moov(&original, &[None, Some(edts.clone())])
            .unwrap()
            .unwrap();
        assert_eq!(patched.len(), original.len() + edts.len());
        let payload = &patched[8..];
        let traks: Vec<_> = boxes(payload)
            .unwrap()
            .into_iter()
            .filter(|(k, _, _)| k == b"trak")
            .collect();
        let count = |i: usize| boxes(&payload[traks[i].1 + 8..traks[i].2]).unwrap().len();
        assert_eq!((count(1), count(2)), (2, 3));
    }

    /// A full-size `mvhd` / `tkhd` with a recognisable duration field.
    fn header_box(kind: &[u8; 4], version: u8, duration: u64) -> Vec<u8> {
        let mut payload = vec![version, 0, 0, 0];
        let times = if version == 1 { 16 } else { 8 };
        payload.extend(vec![0u8; times]); // creation + modification
        payload.extend_from_slice(&[0, 0, 0, 1]); // timescale (mvhd) / track id (tkhd)
        if kind == b"tkhd" {
            payload.extend_from_slice(&[0, 0, 0, 0]); // reserved
        }
        if version == 1 {
            payload.extend_from_slice(&duration.to_be_bytes());
        } else {
            payload.extend_from_slice(&(duration as u32).to_be_bytes());
        }
        payload.extend(vec![0u8; 40]); // the rest of the header
        make_box(kind, &payload)
    }

    fn duration_of(b: &[u8]) -> u64 {
        let version = b[8];
        let times = if version == 1 { 16 } else { 8 };
        let at = 12 + times + 4 + if &b[4..8] == b"tkhd" { 4 } else { 0 };
        if version == 1 {
            u64::from_be_bytes(b[at..at + 8].try_into().unwrap())
        } else {
            u64::from(u32::from_be_bytes(b[at..at + 4].try_into().unwrap()))
        }
    }

    #[test]
    fn durations_are_rewritten_in_both_header_versions() {
        for kind in [b"mvhd", b"tkhd"] {
            for version in [0u8, 1] {
                let mut b = header_box(kind, version, 777);
                assert!(set_box_duration(&mut b, 123_456));
                assert_eq!(duration_of(&b), 123_456, "{kind:?} v{version}");
                // Nothing else moved.
                assert_eq!(b.len(), header_box(kind, version, 0).len());
            }
        }
    }

    #[test]
    fn a_box_too_short_for_the_field_is_left_alone() {
        let mut short = make_box(b"mvhd", &[0; 6]);
        let before = short.clone();
        assert!(!set_box_duration(&mut short, 5));
        assert_eq!(short, before);
        let mut other = make_box(b"free", &[0; 60]);
        assert!(!set_box_duration(&mut other, 5));
    }

    #[test]
    fn a_video_edit_list_and_the_duration_go_to_the_video_track_only() {
        let video_tkhd = header_box(b"tkhd", 0, 999);
        let sound_tkhd = header_box(b"tkhd", 0, 888);
        let hdlr_box =
            |h: &[u8; 4]| make_box(b"mdia", &[make_box(b"mdhd", &[0; 8]), hdlr(h)].concat());
        let video_trak = make_box(b"trak", &[video_tkhd, hdlr_box(b"vide")].concat());
        let sound_trak = make_box(b"trak", &[sound_tkhd, hdlr_box(b"soun")].concat());
        let original = make_box(
            b"moov",
            &[header_box(b"mvhd", 0, 5000), video_trak, sound_trak].concat(),
        );

        let video_edts = edts_box(&[EditEntry {
            segment_duration: 4000,
            media_time: 1000,
        }]);
        let patched = patch_moov_with(
            &original,
            &MoovPatch {
                video_edts: Some(&video_edts),
                sound_edts: &[],
                movie_duration: Some(4000),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(patched.len(), original.len() + video_edts.len());
        assert_eq!(
            u32::from_be_bytes(patched[..4].try_into().unwrap()) as usize,
            patched.len()
        );

        let payload = &patched[8..];
        let top = boxes(payload).unwrap();
        let (_, s, e) = top[0];
        assert_eq!(duration_of(&payload[s..e]), 4000, "mvhd");
        let traks: Vec<_> = top.iter().filter(|(k, _, _)| k == b"trak").collect();
        let children = |&&(_, s, e): &&([u8; 4], usize, usize)| boxes(&payload[s + 8..e]).unwrap();
        let video_children = children(&traks[0]);
        assert_eq!(
            video_children.iter().map(|c| c.0).collect::<Vec<_>>(),
            [*b"tkhd", *b"edts", *b"mdia"]
        );
        let (_, s, e) = video_children[0];
        let video_body = &payload[traks[0].1 + 8..traks[0].2];
        assert_eq!(duration_of(&video_body[s..e]), 4000, "video tkhd");
        // The sound track is untouched: same children, same duration.
        let sound_children = children(&traks[1]);
        assert_eq!(
            sound_children.iter().map(|c| c.0).collect::<Vec<_>>(),
            [*b"tkhd", *b"mdia"]
        );
        let (_, s, e) = sound_children[0];
        let sound_body = &payload[traks[1].1 + 8..traks[1].2];
        assert_eq!(duration_of(&sound_body[s..e]), 888, "sound tkhd");
    }

    #[test]
    fn a_duration_alone_counts_as_a_change() {
        let original = moov(&[trak(b"vide", 1)]);
        let original = {
            // swap the 20-byte test mvhd for a real one
            let payload = [header_box(b"mvhd", 0, 10), original[8 + 28..].to_vec()].concat();
            make_box(b"moov", &payload)
        };
        let patched = patch_moov_with(
            &original,
            &MoovPatch {
                movie_duration: Some(42),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(patched.is_some());
    }

    #[test]
    fn nothing_to_patch_means_no_change() {
        let original = moov(&[trak(b"vide", 1), trak(b"soun", 2)]);
        assert!(patch_moov(&original, &[None]).unwrap().is_none());
        assert!(patch_moov(&original, &[]).unwrap().is_none());
    }
}
