//! Bilinear scaling of BGRA frames, for sources that capture at the display's
//! own size while the encoder wants a fixed one (a 1440p or 4K screen into
//! 1080p). Pure CPU code, tested on every OS; large frames are split into bands
//! of rows that run on several threads.

use std::thread;

/// Below this many output pixels one thread does all the work: spawning costs
/// more than it saves.
const PARALLEL_MIN_PIXELS: usize = 256 * 256;

/// One output column: which two source columns it blends, and how much of the
/// second (0..=256).
#[derive(Clone, Copy)]
struct Tap {
    x0: usize,
    x1: usize,
    weight: u32,
}

/// Where output index `i` of `out_len` samples falls in a source of `in_len`
/// samples, as (first, second, weight of the second in 0..=256). Pixel centers
/// are aligned, so halving a frame averages every 2×2 block.
fn tap(i: usize, in_len: usize, out_len: usize) -> (usize, usize, u32) {
    // position in 1/256ths of a source sample
    let pos = ((2 * i + 1) * in_len * 256) as i64 / (2 * out_len) as i64 - 128;
    let pos = pos.clamp(0, ((in_len - 1) * 256) as i64) as usize;
    let first = pos / 256;
    let second = (first + 1).min(in_len - 1);
    (first, second, (pos % 256) as u32)
}

/// Scale the BGRA picture in `src` (`src_w`×`src_h`, `src_stride` bytes per
/// row) to `dst` (`dst_w`×`dst_h`, tightly packed). Any size to any size.
///
/// Panics if a buffer is too small for its dimensions.
pub fn scale_bgra(
    src: &[u8],
    src_w: usize,
    src_h: usize,
    src_stride: usize,
    dst: &mut [u8],
    dst_w: usize,
    dst_h: usize,
) {
    assert!(
        src_w > 0 && src_h > 0 && dst_w > 0 && dst_h > 0,
        "empty frame"
    );
    assert!(src_stride >= src_w * 4, "stride shorter than a row");
    assert!(
        src.len() >= src_stride * (src_h - 1) + src_w * 4,
        "source too small"
    );
    assert!(dst.len() >= dst_w * dst_h * 4, "destination too small");

    let dst = &mut dst[..dst_w * dst_h * 4];
    if src_w == dst_w && src_h == dst_h {
        for (row, out) in dst.chunks_exact_mut(dst_w * 4).enumerate() {
            out.copy_from_slice(&src[row * src_stride..row * src_stride + dst_w * 4]);
        }
        return;
    }

    let taps: Vec<Tap> = (0..dst_w)
        .map(|x| {
            let (x0, x1, weight) = tap(x, src_w, dst_w);
            Tap { x0, x1, weight }
        })
        .collect();

    let band = |first_row: usize, out: &mut [u8]| {
        for (i, out_row) in out.chunks_exact_mut(dst_w * 4).enumerate() {
            let (y0, y1, wy) = tap(first_row + i, src_h, dst_h);
            let top = &src[y0 * src_stride..y0 * src_stride + src_w * 4];
            let bottom = &src[y1 * src_stride..y1 * src_stride + src_w * 4];
            for (pixel, t) in out_row.chunks_exact_mut(4).zip(&taps) {
                for c in 0..4 {
                    let a = top[t.x0 * 4 + c] as u32 * (256 - t.weight)
                        + top[t.x1 * 4 + c] as u32 * t.weight;
                    let b = bottom[t.x0 * 4 + c] as u32 * (256 - t.weight)
                        + bottom[t.x1 * 4 + c] as u32 * t.weight;
                    pixel[c] = ((a * (256 - wy) + b * wy + (1 << 15)) >> 16) as u8;
                }
            }
        }
    };

    let threads = if dst_w * dst_h < PARALLEL_MIN_PIXELS {
        1
    } else {
        thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8)
    };
    if threads <= 1 {
        band(0, dst);
        return;
    }
    let rows_per_band = dst_h.div_ceil(threads);
    thread::scope(|scope| {
        for (n, out) in dst.chunks_mut(rows_per_band * dst_w * 4).enumerate() {
            let band = &band;
            scope.spawn(move || band(n * rows_per_band, out));
        }
    });
}

/// The size to capture at: the screen's own size, shrunk (keeping its shape) to
/// fit inside `max_w`×`max_h`, never enlarged, and made even — video encoders
/// want even sides.
pub fn fit_even(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let (mut w, mut h) = (src_w.max(2), src_h.max(2));
    if w > max_w || h > max_h {
        // w/src_w and h/src_h are the two ratios; the smaller one wins
        let by_width = u64::from(max_w) * u64::from(src_h) <= u64::from(max_h) * u64::from(src_w);
        if by_width {
            w = max_w;
            h = (u64::from(src_h) * u64::from(max_w) / u64::from(src_w)) as u32;
        } else {
            h = max_h;
            w = (u64::from(src_w) * u64::from(max_h) / u64::from(src_h)) as u32;
        }
    }
    ((w & !1).max(2), (h & !1).max(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, bgra: [u8; 4]) -> Vec<u8> {
        bgra.iter().copied().cycle().take(w * h * 4).collect()
    }

    fn pixel(buf: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let at = (y * w + x) * 4;
        buf[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn the_same_size_is_a_copy_that_drops_the_row_padding() {
        // 3 pixels wide, 8 bytes of padding per row
        let stride = 3 * 4 + 8;
        let mut src = vec![0xEE; stride * 2];
        for (y, row) in src.chunks_mut(stride).enumerate() {
            for (x, p) in row[..12].chunks_mut(4).enumerate() {
                p.copy_from_slice(&[x as u8, y as u8, 7, 255]);
            }
        }
        let mut dst = vec![0u8; 3 * 2 * 4];
        scale_bgra(&src, 3, 2, stride, &mut dst, 3, 2);
        assert_eq!(pixel(&dst, 3, 2, 1), [2, 1, 7, 255]);
        assert!(!dst.contains(&0xEE), "no padding leaks into the picture");
    }

    #[test]
    fn a_flat_color_stays_that_color_at_any_size() {
        let color = [10, 130, 250, 255];
        for (sw, sh, dw, dh) in [
            (64, 48, 32, 24),
            (33, 17, 100, 61),
            (1920, 1080, 1280, 720),
            (5, 5, 1, 1),
        ] {
            let src = solid(sw, sh, color);
            let mut dst = vec![0u8; dw * dh * 4];
            scale_bgra(&src, sw, sh, sw * 4, &mut dst, dw, dh);
            assert!(
                dst.chunks_exact(4).all(|p| p == color),
                "{sw}x{sh} -> {dw}x{dh}"
            );
        }
    }

    #[test]
    fn halving_averages_each_two_by_two_block() {
        // columns alternate 0 and 200: every output pixel is their mean
        let (w, h) = (8, 4);
        let mut src = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if x % 2 == 0 { 0 } else { 200 };
                src[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let mut dst = vec![0u8; 4 * 2 * 4];
        scale_bgra(&src, w, h, w * 4, &mut dst, 4, 2);
        for p in dst.chunks_exact(4) {
            assert_eq!(p, [100, 100, 100, 255]);
        }
    }

    #[test]
    fn a_gradient_stays_in_order_and_inside_its_range() {
        let (w, h) = (200, 10);
        let mut src = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                src[(y * w + x) * 4] = x as u8; // blue ramps 0..199
            }
        }
        let mut dst = vec![0u8; 50 * 10 * 4];
        scale_bgra(&src, w, h, w * 4, &mut dst, 50, 10);
        let blues: Vec<u8> = (0..50).map(|x| pixel(&dst, 50, x, 3)[0]).collect();
        assert!(blues.windows(2).all(|p| p[0] <= p[1]), "{blues:?}");
        assert!(blues[0] < 5 && blues[49] > 194, "{blues:?}");
    }

    #[test]
    fn enlarging_works_too_and_corners_keep_their_colors() {
        let mut src = vec![0u8; 2 * 2 * 4];
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
        ];
        for (i, c) in colors.iter().enumerate() {
            src[i * 4..][..4].copy_from_slice(c);
        }
        let mut dst = vec![0u8; 8 * 8 * 4];
        scale_bgra(&src, 2, 2, 8, &mut dst, 8, 8);
        assert_eq!(pixel(&dst, 8, 0, 0), colors[0]);
        assert_eq!(pixel(&dst, 8, 7, 0), colors[1]);
        assert_eq!(pixel(&dst, 8, 0, 7), colors[2]);
        assert_eq!(pixel(&dst, 8, 7, 7), colors[3]);
    }

    #[test]
    fn big_frames_use_the_threads_and_give_the_same_picture() {
        // Same input through the one-band path (small enough) and the parallel
        // one must agree: scale a block of a large frame both ways.
        let (sw, sh, dw, dh) = (1280, 720, 640, 360);
        let mut src = vec![0u8; sw * sh * 4];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i.wrapping_mul(2654435761) >> 7) as u8;
        }
        let mut parallel = vec![0u8; dw * dh * 4];
        scale_bgra(&src, sw, sh, sw * 4, &mut parallel, dw, dh);

        let mut serial = vec![0u8; dw * dh * 4];
        for (n, out) in serial.chunks_mut(dw * 4).enumerate() {
            // one row at a time: a single-row call is the serial path
            let (y0, y1, wy) = tap(n, sh, dh);
            let mut row = vec![0u8; dw * 4];
            let mut two = Vec::new();
            two.extend_from_slice(&src[y0 * sw * 4..(y0 + 1) * sw * 4]);
            two.extend_from_slice(&src[y1 * sw * 4..(y1 + 1) * sw * 4]);
            // reproduce the vertical blend by hand
            let mut tmp = vec![0u8; dw * 4 * 2];
            scale_bgra(&two[..sw * 4], sw, 1, sw * 4, &mut tmp[..dw * 4], dw, 1);
            scale_bgra(&two[sw * 4..], sw, 1, sw * 4, &mut tmp[dw * 4..], dw, 1);
            for i in 0..dw * 4 {
                let a = u32::from(tmp[i]);
                let b = u32::from(tmp[dw * 4 + i]);
                row[i] = ((a * (256 - wy) + b * wy + 128) >> 8) as u8;
            }
            out.copy_from_slice(&row);
        }
        let worst = parallel
            .iter()
            .zip(&serial)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 1, "rounding only, got {worst}");
    }

    #[test]
    fn the_capture_size_fits_the_limit_keeps_the_shape_and_is_even() {
        assert_eq!(fit_even(1920, 1080, 1920, 1080), (1920, 1080));
        assert_eq!(fit_even(2560, 1440, 1920, 1080), (1920, 1080));
        assert_eq!(fit_even(3840, 2160, 1920, 1080), (1920, 1080));
        // a smaller screen is not enlarged
        assert_eq!(fit_even(1366, 768, 1920, 1080), (1366, 768));
        assert_eq!(fit_even(1024, 768, 1920, 1080), (1024, 768));
        // 21:9 and 16:10 keep their shape
        assert_eq!(fit_even(3440, 1440, 1920, 1080), (1920, 802));
        assert_eq!(fit_even(2560, 1600, 1920, 1080), (1728, 1080));
        // odd sides become even
        assert_eq!(fit_even(1365, 767, 1920, 1080), (1364, 766));
        // never zero
        assert_eq!(fit_even(1, 1, 1920, 1080), (2, 2));
    }

    #[test]
    #[should_panic(expected = "destination too small")]
    fn a_short_destination_is_a_bug_not_a_silent_crop() {
        let src = solid(4, 4, [1, 2, 3, 4]);
        scale_bgra(&src, 4, 4, 16, &mut [0u8; 8], 2, 2);
    }
}
