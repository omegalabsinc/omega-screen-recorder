use crate::error::{Result, ScreenRecError};
use scrap::{Capturer, Display};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub timestamp: Duration,
}

pub struct ScreenCapture {
    capturer: Capturer,
    width: usize,
    height: usize,
    fps: u32,
}

impl ScreenCapture {
    pub fn new(display_index: usize, fps: u32) -> Result<Self> {
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

        let capturer = Capturer::new(*display).map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to create capturer: {}", e))
        })?;

        let width = capturer.width();
        let height = capturer.height();

        log::info!("Screen capture initialized: {}x{} @ {}fps", width, height, fps);

        Ok(Self {
            capturer,
            width,
            height,
            fps,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Start capturing frames and send them through the channel
    pub async fn start_capture(
        mut self,
        tx: mpsc::Sender<Frame>,
        duration: Option<Duration>,
    ) -> Result<()> {
        let frame_duration = Duration::from_micros(1_000_000 / self.fps as u64);
        let start_time = Instant::now();
        let mut frame_count = 0u64;

        log::info!("Starting screen capture...");

        loop {
            let frame_start = Instant::now();

            // Check if we should stop (duration limit reached)
            if let Some(dur) = duration {
                if start_time.elapsed() >= dur {
                    log::info!("Capture duration reached");
                    break;
                }
            }

            // Capture frame
            match self.capturer.frame() {
                Ok(frame) => {
                    // Convert BGRA to RGB (removing alpha channel for better compression)
                    let mut rgb_data = Vec::with_capacity(self.width * self.height * 3);
                    for chunk in frame.chunks_exact(4) {
                        rgb_data.push(chunk[2]); // R
                        rgb_data.push(chunk[1]); // G
                        rgb_data.push(chunk[0]); // B
                    }

                    let captured_frame = Frame {
                        data: rgb_data,
                        width: self.width,
                        height: self.height,
                        timestamp: start_time.elapsed(),
                    };

                    // Send frame through channel
                    if tx.send(captured_frame).await.is_err() {
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
                    tokio::time::sleep(Duration::from_millis(1)).await;
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
                tokio::time::sleep(frame_duration - elapsed).await;
            }
        }

        log::info!("Screen capture finished. Total frames: {}", frame_count);
        Ok(())
    }
}
