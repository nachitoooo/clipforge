//! One still of the primary monitor through Windows Graphics Capture, saved as a
//! PNG. Never run on a real desktop outside CI; see `windows_video` tests.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::time::Duration;

use crate::capture::windows::capture_one_frame;
use crate::capture::CaptureError;

use super::Screenshot;

/// How long to wait for the screen to hand over its first picture.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(3);

pub fn capture_png(path: &Path) -> Result<Screenshot, CaptureError> {
    let (bgra, width, height) = capture_one_frame(FIRST_FRAME_TIMEOUT)?;
    write_png(path, &bgra, width, height)?;
    Ok(Screenshot { width, height })
}

/// Save tightly packed BGRA pixels as an opaque RGB PNG.
pub(crate) fn write_png(
    path: &Path,
    bgra: &[u8],
    width: u32,
    height: u32,
) -> Result<(), CaptureError> {
    let fail = |e: &dyn std::fmt::Display| {
        CaptureError::Backend(format!("no se pudo guardar la captura: {e}"))
    };
    let mut rgb = Vec::with_capacity(bgra.len() / 4 * 3);
    for pixel in bgra.as_chunks::<4>().0 {
        rgb.extend([pixel[2], pixel[1], pixel[0]]);
    }
    let file = File::create(path).map_err(|e| fail(&e))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    // The default level is slow for a 4K picture; this one is much faster for a
    // modest size cost.
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| fail(&e))?;
    writer.write_image_data(&rgb).map_err(|e| fail(&e))?;
    writer.finish().map_err(|e| fail(&e))
}
