use crate::error::{Result, ScreenRecError};
use image::{ImageBuffer, RgbaImage};
use scrap::{Capturer, Display};
use std::path::Path;

pub fn capture_screenshot(output_path: &Path, display_index: usize) -> Result<()> {
    log::info!("Capturing screenshot to: {:?}", output_path);

    // Get the specified display
    let displays = Display::all().map_err(|e| {
        ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
    })?;

    if displays.is_empty() {
        return Err(ScreenRecError::CaptureError(
            "No displays found".to_string(),
        ));
    }

    let display = displays.get(display_index).ok_or_else(|| {
        ScreenRecError::CaptureError(format!("Display {} not found", display_index))
    })?;

    log::info!("Display dimensions: {}x{}", display.width(), display.height());

    // Create capturer
    let mut capturer = Capturer::new(*display).map_err(|e| {
        ScreenRecError::CaptureError(format!("Failed to create capturer: {}", e))
    })?;

    let width = capturer.width();
    let height = capturer.height();

    // Capture frame
    let frame = loop {
        match capturer.frame() {
            Ok(frame) => break frame,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // Frame not ready yet, wait a bit
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            Err(e) => {
                return Err(ScreenRecError::CaptureError(format!(
                    "Failed to capture frame: {}",
                    e
                )))
            }
        }
    };

    // Convert BGRA to RGBA
    let mut rgba_data = Vec::with_capacity(width * height * 4);
    for chunk in frame.chunks_exact(4) {
        rgba_data.push(chunk[2]); // R
        rgba_data.push(chunk[1]); // G
        rgba_data.push(chunk[0]); // B
        rgba_data.push(chunk[3]); // A
    }

    // Create image buffer
    let img: RgbaImage = ImageBuffer::from_raw(width as u32, height as u32, rgba_data)
        .ok_or_else(|| ScreenRecError::CaptureError("Failed to create image buffer".to_string()))?;

    // Determine output format from extension
    let extension = output_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();

    // Save image
    match extension.as_str() {
        "png" => img.save(output_path)?,
        "jpg" | "jpeg" => {
            let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();
            rgb_img.save(output_path)?;
        }
        _ => {
            return Err(ScreenRecError::InvalidParameter(format!(
                "Unsupported image format: {}. Use .png, .jpg, or .jpeg",
                extension
            )))
        }
    }

    log::info!("Screenshot saved successfully");
    Ok(())
}
