use crate::error::{Result, ScreenRecError};
use chrono::{DateTime, Utc};
use scrap::{Capturer, Display};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub timestamp: Duration,
    pub captured_at: DateTime<Utc>,
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
    pub fn start_capture_sync(
        self,
        tx: std::sync::mpsc::Sender<Frame>,
        target_frames: Option<u64>,
    ) -> Result<()> {
        // Create capturer inside this thread (can't be moved between threads)
        let displays = Display::all().map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
        })?;

        if self.display_index >= displays.len() {
            return Err(ScreenRecError::CaptureError(format!(
                "Display {} not found (only {} displays available)",
                self.display_index,
                displays.len()
            )));
        }

        let display = displays
            .into_iter()
            .nth(self.display_index)
            .ok_or_else(|| {
                ScreenRecError::CaptureError(format!("Display {} not found", self.display_index))
            })?;

        let mut capturer = Capturer::new(display).map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to create capturer: {}", e))
        })?;

        let width = capturer.width();
        let height = capturer.height();

        let frame_duration = Duration::from_micros(1_000_000 / self.fps as u64);
        let mut start_time: Option<Instant> = None;
        let mut frame_count = 0u64;

        log::info!("Starting screen capture...");
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
