use crate::error::{Result, ScreenRecError};
use chrono::{DateTime, Utc};
use scrap::{Capturer, Display};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use core_graphics::display::{CGDisplay, CGPoint};

/// Detects which display the cursor is currently on
pub fn get_cursor_display() -> Result<usize> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            get_cursor_display_macos()
        } else if #[cfg(target_os = "windows")] {
            get_cursor_display_windows()
        } else {
            // Linux/others - default to display 0
            log::warn!("Cursor display detection not implemented for this platform, using display 0");
            Ok(0)
        }
    }
}

#[cfg(target_os = "macos")]
fn get_cursor_display_macos() -> Result<usize> {
    use core_graphics::display::CGGetActiveDisplayList;

    // Get cursor position using CGEvent
    use core_graphics::event::{CGEvent};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    let event_source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|e| ScreenRecError::CaptureError(format!("Failed to create event source: {:?}", e)))?;

    let event = CGEvent::new(event_source)
        .map_err(|_| ScreenRecError::CaptureError("Failed to create event".to_string()))?;

    let cursor_location = event.location();

    // Get all active displays
    let mut display_ids = vec![0u32; 16];
    let mut display_count = 0u32;

    unsafe {
        CGGetActiveDisplayList(display_ids.len() as u32, display_ids.as_mut_ptr(), &mut display_count);
    }

    // Find which display contains the cursor
    for (index, &display_id) in display_ids.iter().take(display_count as usize).enumerate() {
        let cg_display = CGDisplay::new(display_id);
        let bounds = cg_display.bounds();

        // Check if cursor is within this display's bounds
        if cursor_location.x >= bounds.origin.x
            && cursor_location.x < bounds.origin.x + bounds.size.width
            && cursor_location.y >= bounds.origin.y
            && cursor_location.y < bounds.origin.y + bounds.size.height
        {
            log::info!("Cursor detected on display {} at ({}, {})", index, cursor_location.x, cursor_location.y);
            return Ok(index);
        }
    }

    // Default to first display if cursor not found
    log::warn!("Could not determine cursor display, using display 0");
    Ok(0)
}

#[cfg(target_os = "windows")]
fn get_cursor_display_windows() -> Result<usize> {
    // TODO: Implement Windows cursor detection
    log::warn!("Cursor display detection not yet implemented for Windows, using display 0");
    Ok(0)
}

#[derive(Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub timestamp: Duration,
    pub captured_at: DateTime<Utc>,
    pub display_id: usize,
}


pub struct ScreenCapture {
    display_index: usize,
    fps: u32,
}

impl ScreenCapture {
    pub fn new(display_index: usize, fps: u32) -> Result<Self> {
        // Just validate that the display exists
        let displays = Display::all().map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
        })?;

        if displays.is_empty() {
            return Err(ScreenRecError::CaptureError(
                "No displays found".to_string(),
            ));
        }

        if display_index >= displays.len() {
            return Err(ScreenRecError::CaptureError(format!(
                "Display {} not found (only {} displays available)",
                display_index,
                displays.len()
            )));
        }

        log::info!(
            "Screen capture configured for display {} @ {}fps",
            display_index,
            fps
        );

        Ok(Self { display_index, fps })
    }

    fn get_display_size(&self) -> Result<(usize, usize)> {
        let displays = Display::all().map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
        })?;

        let display = displays.get(self.display_index).ok_or_else(|| {
            ScreenRecError::CaptureError(format!("Display {} not found", self.display_index))
        })?;

        Ok((display.width(), display.height()))
    }

    pub fn width(&self) -> usize {
        self.get_display_size().map(|(w, _)| w).unwrap_or(1920)
    }

    pub fn height(&self) -> usize {
        self.get_display_size().map(|(_, h)| h).unwrap_or(1080)
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Start capturing frames and send them through the channel
    /// This runs synchronously in a blocking thread
    /// Dynamically follows cursor to different displays
    pub fn start_capture_sync(
        mut self,
        tx: std::sync::mpsc::Sender<Frame>,
        target_frames: Option<u64>,
    ) -> Result<()> {
        // Helper function to create capturer for a display
        let create_capturer = |display_index: usize| -> Result<(Capturer, usize, usize)> {
            let displays = Display::all().map_err(|e| {
                ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
            })?;

            if display_index >= displays.len() {
                return Err(ScreenRecError::CaptureError(format!(
                    "Display {} not found (only {} displays available)",
                    display_index,
                    displays.len()
                )));
            }

            let display = displays
                .into_iter()
                .nth(display_index)
                .ok_or_else(|| {
                    ScreenRecError::CaptureError(format!("Display {} not found", display_index))
                })?;

            let capturer = Capturer::new(display).map_err(|e| {
                ScreenRecError::CaptureError(format!("Failed to create capturer: {}", e))
            })?;

            let width = capturer.width();
            let height = capturer.height();

            Ok((capturer, width, height))
        };

        // Create initial capturer
        let (mut capturer, mut width, mut height) = create_capturer(self.display_index)?;

        let frame_duration = Duration::from_micros(1_000_000 / self.fps as u64);
        let mut start_time: Option<Instant> = None;
        let mut frame_count = 0u64;
        let mut last_cursor_check = Instant::now();
        let cursor_check_interval = Duration::from_millis(100); // Check cursor every 100ms

        log::info!("Starting screen capture with dynamic cursor tracking...");
        log::info!("Waiting for first frame (grant screen recording permission if prompted)...");

        loop {
            let frame_start = Instant::now();

            // Check if we should stop (target frames reached)
            if let Some(target) = target_frames {
                if frame_count >= target {
                    log::info!("Target frames reached: {}/{}", frame_count, target);
                    break;
                }
            }

            // Periodically check if cursor moved to a different display
            if frame_start.duration_since(last_cursor_check) >= cursor_check_interval {
                if let Ok(cursor_display) = get_cursor_display() {
                    if cursor_display != self.display_index {
                        log::info!("Cursor moved from display {} to display {}, switching capture",
                            self.display_index, cursor_display);

                        // Recreate capturer for new display
                        match create_capturer(cursor_display) {
                            Ok((new_capturer, new_width, new_height)) => {
                                capturer = new_capturer;
                                width = new_width;
                                height = new_height;
                                self.display_index = cursor_display;
                            }
                            Err(e) => {
                                log::warn!("Failed to switch to display {}: {}", cursor_display, e);
                            }
                        }
                    }
                }
                last_cursor_check = frame_start;
            }

            // Capture frame
            match capturer.frame() {
                Ok(frame) => {
                    // Convert BGRA to RGB (removing alpha channel for better compression)
                    let mut rgb_data = Vec::with_capacity(width * height * 3);
                    for chunk in frame.chunks_exact(4) {
                        rgb_data.push(chunk[2]); // R
                        rgb_data.push(chunk[1]); // G
                        rgb_data.push(chunk[0]); // B
                    }

                    // Start the timer on first successful frame
                    if start_time.is_none() {
                        start_time = Some(Instant::now());
                        log::info!("First frame captured, recording started!");
                    }

                    let captured_frame = Frame {
                        data: rgb_data,
                        width,
                        height,
                        timestamp: start_time.unwrap().elapsed(),
                        captured_at: Utc::now(),
                        display_id: self.display_index,
                    };

                    // Send frame through channel
                    if tx.send(captured_frame).is_err() {
                        log::warn!("Frame receiver dropped, stopping capture");
                        break;
                    }

                    frame_count += 1;
                    if frame_count % (self.fps as u64) == 0 {
                        log::debug!("Captured {} frames", frame_count);
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Frame not ready yet, wait a bit
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(e) => {
                    log::error!("Frame capture error: {}", e);
                    return Err(ScreenRecError::CaptureError(format!(
                        "Failed to capture frame: {}",
                        e
                    )));
                }
            }

            // Maintain frame rate
            let elapsed = frame_start.elapsed();
            if elapsed < frame_duration {
                std::thread::sleep(frame_duration - elapsed);
            }
        }

        log::info!("Screen capture finished. Total frames: {}", frame_count);
        Ok(())
    }
}
