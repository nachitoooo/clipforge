//! BGRA → NV12, the picture format hardware and Media Foundation H.264
//! encoders take: a full-resolution brightness plane (Y) followed by one plane
//! of interleaved blue/red differences (UV) at half the width and height.
//!
//! BT.709 matrix, limited ("TV") range — the convention players assume for HD
//! video, and what the encoder is told in its input type. Integer maths, so it
//! is deterministic and fast enough for 1080p60 when split into bands over a
//! few threads. Pure code, tested on every OS.

use std::thread;

/// Below this many pixels one thread does it all.
const PARALLEL_MIN_PIXELS: usize = 320 * 240;

/// Bytes an NV12 picture of `width`×`height` takes. Both must be even.
pub fn nv12_len(width: usize, height: usize) -> usize {
    width * height + width * height / 2
}

#[inline]
fn luma(b: i32, g: i32, r: i32) -> u8 {
    (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8
}

#[inline]
fn chroma_u(b: i32, g: i32, r: i32) -> u8 {
    (((-26 * r - 86 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8
}

#[inline]
fn chroma_v(b: i32, g: i32, r: i32) -> u8 {
    (((112 * r - 102 * g - 10 * b + 128) >> 8) + 128).clamp(16, 240) as u8
}

/// Convert the tightly packed BGRA picture `src` (`width`×`height`) into
/// `dst`, which must hold exactly [`nv12_len`] bytes. Alpha is ignored.
///
/// Panics if the sides are odd or zero, or a buffer has the wrong size.
pub fn bgra_to_nv12(src: &[u8], width: usize, height: usize, dst: &mut [u8]) {
    assert!(width >= 2 && height >= 2, "empty picture");
    assert!(
        width.is_multiple_of(2) && height.is_multiple_of(2),
        "NV12 needs even sides"
    );
    assert_eq!(
        src.len(),
        width * height * 4,
        "source is not width*height BGRA"
    );
    assert_eq!(
        dst.len(),
        nv12_len(width, height),
        "destination is not an NV12 picture"
    );

    let (y_plane, uv_plane) = dst.split_at_mut(width * height);

    // A band is a run of row *pairs*: the chroma of rows 2k and 2k+1 is one row.
    let band = |first_pair: usize, y_rows: &mut [u8], uv_rows: &mut [u8]| {
        for (i, (y_pair, uv_row)) in y_rows
            .chunks_exact_mut(width * 2)
            .zip(uv_rows.chunks_exact_mut(width))
            .enumerate()
        {
            let top_at = (first_pair + i) * 2 * width * 4;
            let top = &src[top_at..top_at + width * 4];
            let bottom = &src[top_at + width * 4..top_at + width * 8];
            let (y_top, y_bottom) = y_pair.split_at_mut(width);
            for x in (0..width).step_by(2) {
                let mut sum = [0i32; 3];
                for (row, y_row) in [(top, &mut *y_top), (bottom, &mut *y_bottom)] {
                    for dx in 0..2 {
                        let p = &row[(x + dx) * 4..(x + dx) * 4 + 3];
                        let (b, g, r) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
                        y_row[x + dx] = luma(b, g, r);
                        sum[0] += b;
                        sum[1] += g;
                        sum[2] += r;
                    }
                }
                let (b, g, r) = ((sum[0] + 2) >> 2, (sum[1] + 2) >> 2, (sum[2] + 2) >> 2);
                uv_row[x] = chroma_u(b, g, r);
                uv_row[x + 1] = chroma_v(b, g, r);
            }
        }
    };

    let pairs = height / 2;
    let threads = if width * height < PARALLEL_MIN_PIXELS {
        1
    } else {
        thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8)
    };
    if threads <= 1 {
        band(0, y_plane, uv_plane);
        return;
    }
    let pairs_per_band = pairs.div_ceil(threads);
    thread::scope(|scope| {
        let y_bands = y_plane.chunks_mut(pairs_per_band * 2 * width);
        let uv_bands = uv_plane.chunks_mut(pairs_per_band * width);
        for (n, (y_rows, uv_rows)) in y_bands.zip(uv_bands).enumerate() {
            let band = &band;
            scope.spawn(move || band(n * pairs_per_band, y_rows, uv_rows));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: usize, h: usize, bgra: [u8; 4]) -> Vec<u8> {
        bgra.iter().copied().cycle().take(w * h * 4).collect()
    }

    fn convert(w: usize, h: usize, src: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; nv12_len(w, h)];
        bgra_to_nv12(src, w, h, &mut out);
        out
    }

    /// Floating-point BT.709 limited range, the reference the integer maths
    /// must stay close to.
    fn reference(b: u8, g: u8, r: u8) -> (f64, f64, f64) {
        let (b, g, r) = (
            f64::from(b) / 255.0,
            f64::from(g) / 255.0,
            f64::from(r) / 255.0,
        );
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let u = (b - y) / 1.8556;
        let v = (r - y) / 1.5748;
        (16.0 + 219.0 * y, 128.0 + 224.0 * u, 128.0 + 224.0 * v)
    }

    #[test]
    fn the_size_of_a_picture() {
        assert_eq!(nv12_len(1920, 1080), 1920 * 1080 * 3 / 2);
        assert_eq!(nv12_len(2, 2), 6);
    }

    #[test]
    fn black_and_white_land_on_the_ends_of_the_tv_range_with_neutral_color() {
        let black = convert(4, 4, &picture(4, 4, [0, 0, 0, 255]));
        assert!(black[..16].iter().all(|&y| y == 16));
        assert!(black[16..].iter().all(|&c| c == 128));

        let white = convert(4, 4, &picture(4, 4, [255, 255, 255, 255]));
        assert!(white[..16].iter().all(|&y| y == 235), "{:?}", &white[..4]);
        assert!(white[16..].iter().all(|&c| c == 128));
    }

    #[test]
    fn colors_agree_with_the_floating_point_formula() {
        for (b, g, r) in [
            (0u8, 0u8, 255u8),
            (0, 255, 0),
            (255, 0, 0),
            (255, 255, 0),
            (0, 255, 255),
            (255, 0, 255),
            (128, 128, 128),
            (30, 90, 200),
            (200, 140, 20),
        ] {
            let out = convert(2, 2, &picture(2, 2, [b, g, r, 255]));
            let (y, u, v) = reference(b, g, r);
            assert!(
                (f64::from(out[0]) - y).abs() <= 1.5,
                "Y of {b},{g},{r}: {} vs {y}",
                out[0]
            );
            assert!(
                (f64::from(out[4]) - u).abs() <= 2.0,
                "U of {b},{g},{r}: {} vs {u}",
                out[4]
            );
            assert!(
                (f64::from(out[5]) - v).abs() <= 2.0,
                "V of {b},{g},{r}: {} vs {v}",
                out[5]
            );
        }
    }

    #[test]
    fn alpha_is_ignored() {
        assert_eq!(
            convert(2, 2, &picture(2, 2, [10, 20, 30, 0])),
            convert(2, 2, &picture(2, 2, [10, 20, 30, 255]))
        );
    }

    #[test]
    fn each_pixel_keeps_its_own_brightness_and_the_block_shares_the_color() {
        // 2x2: top row black|white, bottom row white|black
        let mut src = vec![0u8; 16];
        for (i, v) in [0u8, 255, 255, 0].into_iter().enumerate() {
            src[i * 4..][..4].copy_from_slice(&[v, v, v, 255]);
        }
        let out = convert(2, 2, &src);
        assert_eq!(&out[..4], [16, 235, 235, 16], "Y per pixel");
        assert_eq!(&out[4..], [128, 128], "grey average: no color");
    }

    #[test]
    fn the_planes_are_laid_out_as_y_then_interleaved_uv() {
        // 4x2: left block red, right block blue
        let (w, h) = (4, 2);
        let mut src = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let p = if x < 2 {
                    [0, 0, 255, 255]
                } else {
                    [255, 0, 0, 255]
                };
                src[(y * w + x) * 4..][..4].copy_from_slice(&p);
            }
        }
        let out = convert(w, h, &src);
        assert_eq!(out.len(), 12);
        let (u_red, v_red) = (out[8], out[9]);
        let (u_blue, v_blue) = (out[10], out[11]);
        assert!(
            v_red > 200 && u_red < 120,
            "red leans to V: {u_red},{v_red}"
        );
        assert!(
            u_blue > 200 && v_blue < 120,
            "blue leans to U: {u_blue},{v_blue}"
        );
    }

    #[test]
    fn a_big_picture_converted_in_bands_equals_the_same_pixels_converted_alone() {
        // 640x480 goes through the threaded path; its top-left 2x2 block alone
        // goes through the single one. Same pixels, same bytes.
        let (w, h) = (640, 480);
        let mut src = vec![0u8; w * h * 4];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i.wrapping_mul(2654435761) >> 11) as u8;
        }
        let big = convert(w, h, &src);
        for (px, py) in [(0usize, 0usize), (100, 50), (638, 478), (320, 240)] {
            let mut block = Vec::new();
            for dy in 0..2 {
                for dx in 0..2 {
                    let at = ((py + dy) * w + px + dx) * 4;
                    block.extend_from_slice(&src[at..at + 4]);
                }
            }
            let small = convert(2, 2, &block);
            assert_eq!(
                small[..2],
                big[py * w + px..py * w + px + 2],
                "Y row 1 at {px},{py}"
            );
            assert_eq!(
                small[2..4],
                big[(py + 1) * w + px..(py + 1) * w + px + 2],
                "Y row 2 at {px},{py}"
            );
            let uv = w * h + (py / 2) * w + px;
            assert_eq!(small[4..], big[uv..uv + 2], "UV at {px},{py}");
        }
    }

    #[test]
    #[should_panic(expected = "even sides")]
    fn odd_sizes_are_refused() {
        convert(3, 2, &picture(3, 2, [0, 0, 0, 0]));
    }

    #[test]
    #[should_panic(expected = "not width*height")]
    fn a_wrong_sized_source_is_a_bug() {
        let mut out = vec![0u8; nv12_len(2, 2)];
        bgra_to_nv12(&[0u8; 12], 2, 2, &mut out);
    }
}
