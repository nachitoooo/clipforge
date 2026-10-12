//! One still of the first display, through `SCScreenshotManager` (macOS 14+).
//! Needs the same Screen Recording permission as the replay buffer.

use std::path::Path;

use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::{CGImageExt, ImageFormat, SCScreenshotManager};

use crate::capture::CaptureError;

use super::Screenshot;

pub fn capture_png(path: &Path) -> Result<Screenshot, CaptureError> {
    let content = SCShareableContent::get().map_err(map_error)?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or(CaptureError::NoDisplay)?;

    // ClipForge's own windows (this app, its notifications) are not part of the
    // picture, whichever button took it.
    let own = std::process::id() as i32;
    let apps = content.applications();
    let ours: Vec<&SCRunningApplication> =
        apps.iter().filter(|app| app.process_id() == own).collect();
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_applications(&ours, &[])
        .build();
    // The display's size is in points; a Retina screen has 2 pixels per point
    // and the picture should have all of them.
    let scale = filter.point_pixel_scale().max(1.0);
    let config = SCStreamConfiguration::new()
        .with_width((display.width() as f32 * scale).round() as u32)
        .with_height((display.height() as f32 * scale).round() as u32)
        .with_pixel_format(PixelFormat::BGRA)
        .with_shows_cursor(true);

    let image = SCScreenshotManager::capture_image(&filter, &config).map_err(map_error)?;
    let path_text = path
        .to_str()
        .ok_or_else(|| CaptureError::Backend("la ruta de la captura no es texto válido".into()))?;
    image
        .save(path_text, ImageFormat::Png)
        .map_err(|e| CaptureError::Backend(format!("no se pudo guardar la captura: {e}")))?;
    Ok(Screenshot {
        width: image.width() as u32,
        height: image.height() as u32,
    })
}

fn map_error(err: SCError) -> CaptureError {
    match err {
        // A Screen Recording denial is reported as "no shareable content".
        SCError::NoShareableContent(_) => CaptureError::PermissionDenied,
        SCError::DisplayNotFound(_) => CaptureError::NoDisplay,
        other => CaptureError::Backend(other.to_string()),
    }
}
