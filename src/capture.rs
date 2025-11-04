use crate::error::{Result, ScreenRecError};
use scrap::{Capturer, Display};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub timestamp: Duration,
}

impl Frame {
    /// Compare frames and determine if they are similar enough to skip
    /// Returns true if frames are similar (idle frame), false if different
    pub fn is_similar_to(&self, other: &Frame, threshold: f32) -> bool {
        if self.data.len() != other.data.len() {
            return false;
        }

        // Sample-based comparison for performance
        // Check every Nth pixel instead of every pixel
        let sample_rate = 16; // Check every 16th pixel
        let pixels_to_check = self.data.len() / (sample_rate * 3);

        let mut diff_count = 0;
        let max_diff_threshold = 30; // RGB difference threshold per pixel

        for i in 0..pixels_to_check {
            let idx = i * sample_rate * 3;
            if idx + 2 >= self.data.len() {
                break;
            }

            let r_diff = (self.data[idx] as i32 - other.data[idx] as i32).abs();
            let g_diff = (self.data[idx + 1] as i32 - other.data[idx + 1] as i32).abs();
            let b_diff = (self.data[idx + 2] as i32 - other.data[idx + 2] as i32).abs();

            // If any component differs significantly, count it
            if r_diff > max_diff_threshold || g_diff > max_diff_threshold || b_diff > max_diff_threshold {
                diff_count += 1;
            }
        }

        // Calculate percentage of different pixels
        let diff_percentage = (diff_count as f32) / (pixels_to_check as f32);

        // Frames are similar if difference is below threshold
        diff_percentage < threshold
    }
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

        log::info!("Screen capture configured for display {} @ {}fps", display_index, fps);

        Ok(Self {
            display_index,
            fps,
        })
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
    /// Skips idle frames (frames that are very similar to the previous frame) if skip_idle is true
    /// This runs synchronously in a blocking thread
    pub fn start_capture_sync(
        self,
        tx: std::sync::mpsc::Sender<Frame>,
        duration: Option<Duration>,
        skip_idle: bool,
    ) -> Result<()> {
        // Create capturer inside this thread (can't be moved between threads)
        let displays = Display::all().map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
        })?;

        let display = displays.get(self.display_index).ok_or_else(|| {
            ScreenRecError::CaptureError(format!("Display {} not found", self.display_index))
        })?;

        let mut capturer = Capturer::new(*display).map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to create capturer: {}", e))
        })?;

        let width = capturer.width();
        let height = capturer.height();

        let frame_duration = Duration::from_micros(1_000_000 / self.fps as u64);
        let start_time = Instant::now();
        let mut frame_count = 0u64;
        let mut skipped_count = 0u64;
        let mut last_frame: Option<Frame> = None;

        // Idle frame detection settings
        let similarity_threshold = 0.02; // 2% of pixels can differ
        let keyframe_interval = self.fps as u64 * 2; // Force a keyframe every 2 seconds

        if skip_idle {
            log::info!("Starting screen capture with idle frame detection...");
            log::info!("  Similarity threshold: {:.1}%", similarity_threshold * 100.0);
            log::info!("  Keyframe interval: {} frames ({} seconds)", keyframe_interval, keyframe_interval / self.fps as u64);
        } else {
            log::info!("Starting screen capture (idle frame skipping disabled)...");
        }

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
            match capturer.frame() {
                Ok(frame) => {
                    // Convert BGRA to RGB (removing alpha channel for better compression)
                    let mut rgb_data = Vec::with_capacity(width * height * 3);
                    for chunk in frame.chunks_exact(4) {
                        rgb_data.push(chunk[2]); // R
                        rgb_data.push(chunk[1]); // G
                        rgb_data.push(chunk[0]); // B
                    }

                    let captured_frame = Frame {
                        data: rgb_data,
                        width,
                        height,
                        timestamp: start_time.elapsed(),
                    };

                    // Determine if we should send this frame
                    let should_send = if !skip_idle {
                        // Idle frame skipping disabled, send all frames
                        true
                    } else if let Some(ref prev_frame) = last_frame {
                        // Force keyframe at regular intervals
                        let is_keyframe = frame_count % keyframe_interval == 0;

                        // Check if frame is different enough from previous
                        let is_different = !captured_frame.is_similar_to(prev_frame, similarity_threshold);

                        if is_keyframe || is_different {
                            if is_keyframe && !is_different {
                                log::debug!("Sending keyframe (forced) at frame {}", frame_count);
                            }
                            true
                        } else {
                            skipped_count += 1;
                            if skipped_count % (self.fps as u64) == 0 {
                                log::debug!("Skipped {} idle frames (total: {}, sent: {})",
                                    skipped_count, frame_count, frame_count - skipped_count);
                            }
                            false
                        }
                    } else {
                        // Always send first frame
                        true
                    };

                    if should_send {
                        // Send frame through channel
                        if tx.send(captured_frame.clone()).is_err() {
                            log::warn!("Frame receiver dropped, stopping capture");
                            break;
                        }
                        last_frame = Some(captured_frame);
                    }

                    frame_count += 1;
                    if frame_count % (self.fps as u64) == 0 {
                        log::debug!("Captured {} frames ({} sent, {} skipped)",
                            frame_count, frame_count - skipped_count, skipped_count);
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

        let sent_frames = frame_count - skipped_count;
        let skip_percentage = if frame_count > 0 {
            (skipped_count as f64 / frame_count as f64) * 100.0
        } else {
            0.0
        };

        log::info!("Screen capture finished.");
        log::info!("  Total frames captured: {}", frame_count);
        log::info!("  Frames encoded: {}", sent_frames);
        log::info!("  Idle frames skipped: {} ({:.1}%)", skipped_count, skip_percentage);
        Ok(())
    }
}
