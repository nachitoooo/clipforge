//! Adding an `edts`/`elst` edit list to a finished .mp4.
//!
//! An AAC stream decodes with ~2 000 samples of encoder delay, and a clip cut
//! out of the middle of a stream also needs a block of decoder warm-up before
//! its first valid sample. The container's way to say "skip this much at the
//! start" is an edit list; every conforming player honors it. The `mp4` crate
//! can't write one, but its writer always appends `moov` *after* `mdat`, so
//! growing `moov` never shifts a chunk offset. We patch just the tail of the
//! file: find `moov`, insert `edts` into the audio `trak`, fix two sizes.

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

fn handler_is_sound(trak_payload: &[u8]) -> Result<bool, MuxError> {
    for (kind, start, end) in boxes(trak_payload)? {
        if &kind != b"mdia" {
            continue;
        }
        let mdia = &trak_payload[start + 8..end];
        for (kind, s, e) in boxes(mdia)? {
            // hdlr: 8 header + 4 version/flags + 4 pre_defined + 4 handler type
            if &kind == b"hdlr" && e - s >= 20 {
                return Ok(&mdia[s + 16..s + 20] == b"soun");
            }
        }
    }
    Ok(false)
}

/// Insert an `edts` box after the `tkhd` of the sound tracks of a complete
/// `moov` box: `edts_per_sound_track[i]` goes to the i-th sound track (in
/// file order); `None` leaves that track alone. Returns the new `moov`
/// bytes, or `Ok(None)` if nothing needed patching.
pub(crate) fn patch_moov(
    moov: &[u8],
    edts_per_sound_track: &[Option<Vec<u8>>],
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
        let mut edts = None;
        if &kind == b"trak" && handler_is_sound(&child[8..])? {
            edts = edts_per_sound_track
                .get(sound_index)
                .and_then(|e| e.as_deref());
            sound_index += 1;
        }
        let Some(edts) = edts else {
            out.extend_from_slice(child);
            continue;
        };

        let tkhd_end = boxes(&child[8..])?
            .into_iter()
            .find(|(k, _, _)| k == b"tkhd")
            .map(|(_, _, e)| 8 + e)
            .ok_or_else(bad)?;
        let mut trak = Vec::with_capacity(child.len() + edts.len());
        trak.extend_from_slice(&child[..tkhd_end]);
        trak.extend_from_slice(edts);
        trak.extend_from_slice(&child[tkhd_end..]);
        let trak_len = trak.len() as u32;
        trak[..4].copy_from_slice(&trak_len.to_be_bytes());
        out.extend_from_slice(&trak);
        patched_any = true;
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

    let edts: Vec<Option<Vec<u8>>> = per_sound_track
        .iter()
        .map(|entries| (!entries.is_empty()).then(|| edts_box(entries)))
        .collect();
    if let Some(patched) = patch_moov(&moov, &edts)? {
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

    #[test]
    fn nothing_to_patch_means_no_change() {
        let original = moov(&[trak(b"vide", 1), trak(b"soun", 2)]);
        assert!(patch_moov(&original, &[None]).unwrap().is_none());
        assert!(patch_moov(&original, &[]).unwrap().is_none());
    }
}
