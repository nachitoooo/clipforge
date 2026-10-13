//! H.264 "Annex B" byte streams (NAL units separated by `00 00 01` start codes,
//! what Media Foundation's encoder emits) turned into what our packets and the
//! mp4 muxer use: length-prefixed NAL units (AVCC), with the parameter sets
//! (SPS, PPS) taken out to live in the `avcC` box. Pure code, tested on every OS.

use std::fmt;

const NAL_SLICE_MIN: u8 = 1;
const NAL_IDR: u8 = 5;
const NAL_SLICE_MAX: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;

/// One access unit read from an Annex B stream.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessUnit {
    /// The picture's slices, each prefixed with its 4-byte big-endian length.
    /// SPS, PPS, access-unit delimiters and SEI are not in here.
    pub data: Vec<u8>,
    /// An IDR picture: decoding can start here.
    pub is_keyframe: bool,
    /// Parameter sets found in this unit, without start codes.
    pub sps: Vec<Vec<u8>>,
    pub pps: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnexBError {
    /// The bytes don't begin with a start code: not an Annex B stream.
    NoStartCode,
    /// Start codes, but no NAL unit between them.
    Empty,
}

impl fmt::Display for AnnexBError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStartCode => write!(f, "the encoder output has no H.264 start code"),
            Self::Empty => write!(f, "the encoder output has no NAL unit"),
        }
    }
}

impl std::error::Error for AnnexBError {}

/// Where each NAL unit's bytes start and end, excluding start codes and the
/// zero padding that may precede the next one.
fn nal_ranges(stream: &[u8]) -> Result<Vec<(usize, usize)>, AnnexBError> {
    // start of the bytes after each start code, and where that start code began
    let mut starts: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i + 3 <= stream.len() {
        if stream[i] == 0 && stream[i + 1] == 0 && stream[i + 2] == 1 {
            starts.push((i + 3, i));
            i += 3;
        } else {
            i += 1;
        }
    }
    match starts.first() {
        None => return Err(AnnexBError::NoStartCode),
        // Only zero bytes may precede the first start code.
        Some(&(_, at)) if stream[..at].iter().any(|&b| b != 0) => {
            return Err(AnnexBError::NoStartCode)
        }
        Some(_) => {}
    }
    let mut ranges = Vec::new();
    for (n, &(begin, _)) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(stream.len(), |&(_, at)| at);
        // a NAL never ends in 0x00: those bytes are padding (or the zero that
        // makes the next start code four bytes long)
        while end > begin && stream[end - 1] == 0 {
            end -= 1;
        }
        if end > begin {
            ranges.push((begin, end));
        }
    }
    Ok(ranges)
}

/// Read one access unit (what an encoder returns for one input picture).
pub fn parse_access_unit(stream: &[u8]) -> Result<AccessUnit, AnnexBError> {
    let ranges = nal_ranges(stream)?;
    if ranges.is_empty() {
        return Err(AnnexBError::Empty);
    }
    let mut unit = AccessUnit::default();
    // Sized once, exactly: this vector goes into the replay buffer and stays
    // there for minutes, and growing it by doubling would leave up to half of
    // every packet as unused capacity (tens of MB over a full buffer).
    unit.data.reserve_exact(
        ranges
            .iter()
            .filter(|&&(begin, _)| {
                (NAL_SLICE_MIN..=NAL_SLICE_MAX).contains(&(stream[begin] & 0x1F))
            })
            .map(|&(begin, end)| 4 + end - begin)
            .sum(),
    );
    for (begin, end) in ranges {
        let nal = &stream[begin..end];
        match nal[0] & 0x1F {
            NAL_SPS => unit.sps.push(nal.to_vec()),
            NAL_PPS => unit.pps.push(nal.to_vec()),
            kind @ NAL_SLICE_MIN..=NAL_SLICE_MAX => {
                unit.is_keyframe |= kind == NAL_IDR;
                unit.data.extend((nal.len() as u32).to_be_bytes());
                unit.data.extend_from_slice(nal);
            }
            _ => {} // SEI, delimiters, filler: nothing the muxer needs
        }
    }
    Ok(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS: [u8; 4] = [0x67, 0x64, 0x00, 0x28];
    const PPS: [u8; 3] = [0x68, 0xEE, 0x3C];
    const IDR: [u8; 5] = [0x65, 0x88, 0x84, 0x00, 0x33];
    const SLICE: [u8; 4] = [0x41, 0x9A, 0x24, 0x6C];

    fn stream(parts: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (code, nal) in parts {
            out.extend_from_slice(code);
            out.extend_from_slice(nal);
        }
        out
    }

    fn avcc(nals: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for nal in nals {
            out.extend((nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
        out
    }

    const LONG: &[u8] = &[0, 0, 0, 1];
    const SHORT: &[u8] = &[0, 0, 1];

    #[test]
    fn a_keyframe_unit_gives_parameter_sets_apart_and_a_length_prefixed_picture() {
        let unit = parse_access_unit(&stream(&[(LONG, &SPS), (LONG, &PPS), (LONG, &IDR)])).unwrap();
        assert!(unit.is_keyframe);
        assert_eq!(unit.sps, vec![SPS.to_vec()]);
        assert_eq!(unit.pps, vec![PPS.to_vec()]);
        assert_eq!(unit.data, avcc(&[&IDR]));
    }

    #[test]
    fn a_picture_takes_exactly_the_memory_it_needs() {
        // Many slices of different sizes: growing the vector slice by slice would
        // round its capacity up to a power of two.
        let slices: Vec<Vec<u8>> = (1..=9)
            .map(|n| {
                std::iter::once(0x41)
                    .chain((0..n * 777).map(|i| i as u8))
                    .collect()
            })
            .collect();
        let parts: Vec<(&[u8], &[u8])> = std::iter::once((LONG, &SPS[..]))
            .chain(slices.iter().map(|s| (SHORT, s.as_slice())))
            .collect();
        let unit = parse_access_unit(&stream(&parts)).unwrap();
        let wanted: usize = slices.iter().map(|s| 4 + s.len()).sum();
        assert_eq!(unit.data.len(), wanted);
        assert_eq!(
            unit.data.capacity(),
            unit.data.len(),
            "no spare room kept in a packet that sits in the buffer for minutes"
        );
    }

    #[test]
    fn a_plain_slice_is_not_a_keyframe() {
        let unit = parse_access_unit(&stream(&[(LONG, &SLICE)])).unwrap();
        assert!(!unit.is_keyframe);
        assert!(unit.sps.is_empty() && unit.pps.is_empty());
        assert_eq!(unit.data, avcc(&[&SLICE]));
    }

    #[test]
    fn three_and_four_byte_start_codes_mix() {
        let unit =
            parse_access_unit(&stream(&[(SHORT, &SPS), (LONG, &PPS), (SHORT, &IDR)])).unwrap();
        assert_eq!(unit.sps.len(), 1);
        assert_eq!(unit.pps.len(), 1);
        assert_eq!(unit.data, avcc(&[&IDR]));
    }

    #[test]
    fn several_slices_of_one_picture_all_come_through_in_order() {
        let second = [0x41, 0x01, 0x02];
        let unit = parse_access_unit(&stream(&[(LONG, &SLICE), (SHORT, &second)])).unwrap();
        assert_eq!(unit.data, avcc(&[&SLICE, &second]));
    }

    #[test]
    fn delimiters_and_sei_are_left_out() {
        let aud = [0x09, 0xF0];
        let sei = [0x06, 0x05, 0x01, 0xFF, 0x80];
        let unit =
            parse_access_unit(&stream(&[(LONG, &aud), (LONG, &sei), (LONG, &SLICE)])).unwrap();
        assert_eq!(unit.data, avcc(&[&SLICE]));
    }

    #[test]
    fn zero_padding_around_start_codes_is_not_part_of_a_unit() {
        let mut s = vec![0, 0, 0, 0]; // leading zeros
        s.extend_from_slice(&stream(&[(LONG, &IDR)]));
        s.extend([0, 0]); // trailing_zero_8bits
        s.extend(stream(&[(LONG, &SLICE)]));
        let unit = parse_access_unit(&s).unwrap();
        assert_eq!(unit.data, avcc(&[&IDR, &SLICE]));
    }

    #[test]
    fn emulation_prevention_bytes_are_not_mistaken_for_start_codes() {
        // 00 00 03 inside a NAL is escaped data, not a boundary
        let nal = [0x65, 0x00, 0x00, 0x03, 0x01, 0x77];
        let unit = parse_access_unit(&stream(&[(LONG, &nal)])).unwrap();
        assert_eq!(unit.data, avcc(&[&nal]));
    }

    #[test]
    fn what_is_not_annex_b_is_refused() {
        assert_eq!(parse_access_unit(&[]), Err(AnnexBError::NoStartCode));
        // already length-prefixed (AVCC)
        assert_eq!(
            parse_access_unit(&avcc(&[&IDR])),
            Err(AnnexBError::NoStartCode)
        );
        assert_eq!(parse_access_unit(&[0, 0, 0, 1]), Err(AnnexBError::Empty));
        assert_eq!(
            parse_access_unit(&[0, 0, 1, 0, 0, 1]),
            Err(AnnexBError::Empty)
        );
    }

    #[test]
    fn the_prefix_is_the_length_of_the_nal() {
        let unit = parse_access_unit(&stream(&[(LONG, &IDR)])).unwrap();
        assert_eq!(&unit.data[..4], [0, 0, 0, IDR.len() as u8]);
        assert_eq!(unit.data.len(), 4 + IDR.len());
    }
}
