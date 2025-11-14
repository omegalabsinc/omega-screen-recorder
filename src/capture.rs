use crate::display_info::{get_all_displays_with_bounds, get_display_at_cursor, DisplayInfo};
use crate::error::{Result, ScreenRecError};
use chrono::{DateTime, Utc};
use scrap::{Capturer, Display};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Frame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    #[allow(dead_code)]
    pub timestamp: Duration,
    pub captured_at: DateTime<Utc>,
    pub display_index: usize,
}

struct MonitorSwitchDetector {
    check_interval: Duration,
    last_check: Instant,
    current_display: usize,
    pending_display: Option<usize>,
    pending_count: u8,
    #[allow(dead_code)]
    displays_info: Vec<DisplayInfo>,
}

impl MonitorSwitchDetector {
    fn new(check_interval: Duration, initial_display: usize) -> Result<Self> {
        let displays_info = get_all_displays_with_bounds()?;

        Ok(Self {
            check_interval,
            last_check: Instant::now(),
            current_display: initial_display,
            pending_display: None,
            pending_count: 0,
            displays_info,
        })
    }

    /// Check if we should switch displays. Returns Some(new_display_index) if a switch should occur.
    fn check_for_switch(&mut self) -> Option<usize> {
        // Only check at specified intervals
        if self.last_check.elapsed() < self.check_interval {
            return None;
        }

        self.last_check = Instant::now();

        // Get current cursor position
        let (cursor_x, cursor_y) = get_cursor_position()?;

        // Determine which display the cursor is on
        let cursor_display = match get_display_at_cursor(cursor_x, cursor_y) {
            Ok(idx) => idx,
            Err(_) => {
                // If we can't determine display, keep current
                return None;
            }
        };

        // If cursor is on same display, reset pending
        if cursor_display == self.current_display {
            self.pending_display = None;
            self.pending_count = 0;
            return None;
        }

        // If cursor is on a new display
        if Some(cursor_display) == self.pending_display {
            // Same pending display, increment count
            self.pending_count += 1;

            // If we've seen it 2+ times, switch
            if self.pending_count >= 2 {
                log::info!("Switching from display {} to display {}", self.current_display, cursor_display);
                self.current_display = cursor_display;
                self.pending_display = None;
                self.pending_count = 0;
                return Some(cursor_display);
            }
        } else {
            // Different pending display, start new pending
            self.pending_display = Some(cursor_display);
            self.pending_count = 1;
        }

        None
    }
}

pub struct ScreenCapture {
    display_index: usize,
    fps: u32,
    multi_monitor: bool,
    monitor_switch_interval: Duration,
}

impl ScreenCapture {
    pub fn new(display_index: usize, fps: u32, monitor_switch_interval: Duration) -> Result<Self> {
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

        // Check if multi-monitor mode should be enabled
        let multi_monitor = displays.len() > 1;

        if multi_monitor {
            log::info!(
                "Screen capture configured for {} displays @ {}fps with multi-monitor tracking (check interval: {:.1}s)",
                displays.len(),
                fps,
                monitor_switch_interval.as_secs_f64()
            );
        } else {
            log::info!(
                "Screen capture configured for display {} @ {}fps (single monitor)",
                display_index,
                fps
            );
        }

        Ok(Self {
            display_index,
            fps,
            multi_monitor,
            monitor_switch_interval,
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

    #[allow(dead_code)]
    pub fn fps(&self) -> u32 {
        self.fps
    }

    pub fn is_multi_monitor(&self) -> bool {
        self.multi_monitor
    }

    /// Get maximum dimensions across all displays (for encoder initialization)
    pub fn get_max_dimensions(&self) -> Result<(usize, usize)> {
        let displays = Display::all().map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
        })?;

        let (mut max_width, mut max_height) = (0, 0);
        for display in displays {
            let w = display.width();
            let h = display.height();
            max_width = max_width.max(w);
            max_height = max_height.max(h);
        }

        Ok((max_width, max_height))
    }

    /// Start capturing frames and send them through the channel
    /// This runs synchronously in a blocking thread
    pub fn start_capture_sync(
        self,
        tx: std::sync::mpsc::Sender<Frame>,
        target_frames: Option<u64>,
        running: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<()> {
        // Branch based on whether multi-monitor is enabled
        if self.multi_monitor {
            self.start_capture_multi_monitor(tx, target_frames, running)
        } else {
            self.start_capture_single_monitor(tx, target_frames, running)
        }
    }

    /// Single monitor capture path (original implementation, zero overhead)
    fn start_capture_single_monitor(
        self,
        tx: std::sync::mpsc::Sender<Frame>,
        target_frames: Option<u64>,
        running: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
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

            // Check if we should stop (Ctrl+C pressed)
            if let Some(ref running_flag) = running {
                if !running_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    log::info!("Stop signal received, finishing capture...");
                    break;
                }
            }

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

                    // Draw cursor on frame
                    if let Some((cursor_x, cursor_y)) = get_cursor_position() {
                        draw_cursor(&mut rgb_data, width, height, cursor_x, cursor_y);
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
                        display_index: self.display_index,
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

    /// Multi-monitor capture path with cursor-based display switching
    fn start_capture_multi_monitor(
        self,
        tx: std::sync::mpsc::Sender<Frame>,
        target_frames: Option<u64>,
        running: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<()> {
        // Get all displays and create capturers for each
        let displays = Display::all().map_err(|e| {
            ScreenRecError::CaptureError(format!("Failed to enumerate displays: {}", e))
        })?;

        // Determine initial display based on cursor position
        let initial_display = if let Some((cursor_x, cursor_y)) = get_cursor_position() {
            get_display_at_cursor(cursor_x, cursor_y).unwrap_or(self.display_index)
        } else {
            self.display_index
        };

        log::info!("Initializing {} capturers for multi-monitor mode (starting with display {})", displays.len(), initial_display);

        // Create HashMap of capturers
        let mut capturers: HashMap<usize, Capturer> = HashMap::new();
        for (index, display) in displays.into_iter().enumerate() {
            let capturer = Capturer::new(display).map_err(|e| {
                ScreenRecError::CaptureError(format!("Failed to create capturer for display {}: {}", index, e))
            })?;
            capturers.insert(index, capturer);
        }

        // Initialize monitor switch detector
        let mut switch_detector = MonitorSwitchDetector::new(
            self.monitor_switch_interval,
            initial_display
        )?;

        // Start with the initial display
        let mut current_display_index = initial_display;
        let current_capturer = capturers.get_mut(&current_display_index).ok_or_else(|| {
            ScreenRecError::CaptureError(format!("Capturer for display {} not found", current_display_index))
        })?;

        let mut width = current_capturer.width();
        let mut height = current_capturer.height();

        let frame_duration = Duration::from_micros(1_000_000 / self.fps as u64);
        let mut start_time: Option<Instant> = None;
        let mut frame_count = 0u64;

        log::info!("Starting multi-monitor screen capture...");
        log::info!("Waiting for first frame (grant screen recording permission if prompted)...");

        loop {
            let frame_start = Instant::now();

            // Check if we should stop (Ctrl+C pressed)
            if let Some(ref running_flag) = running {
                if !running_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    log::info!("Stop signal received, finishing capture...");
                    break;
                }
            }

            // Check if we should stop (target frames reached)
            if let Some(target) = target_frames {
                if frame_count >= target {
                    log::info!("Target frames reached: {}/{}", frame_count, target);
                    break;
                }
            }

            // Check for monitor switch
            if let Some(new_display) = switch_detector.check_for_switch() {
                if let Some(new_capturer) = capturers.get_mut(&new_display) {
                    current_display_index = new_display;
                    width = new_capturer.width();
                    height = new_capturer.height();
                    log::debug!("Switched to display {} ({}x{})", new_display, width, height);
                }
            }

            // Get current capturer
            let current_capturer = capturers.get_mut(&current_display_index).ok_or_else(|| {
                ScreenRecError::CaptureError(format!("Capturer for display {} not found", current_display_index))
            })?;

            // Capture frame
            match current_capturer.frame() {
                Ok(frame) => {
                    // Convert BGRA to RGB (removing alpha channel for better compression)
                    let mut rgb_data = Vec::with_capacity(width * height * 3);
                    for chunk in frame.chunks_exact(4) {
                        rgb_data.push(chunk[2]); // R
                        rgb_data.push(chunk[1]); // G
                        rgb_data.push(chunk[0]); // B
                    }

                    // Draw cursor on frame
                    if let Some((cursor_x, cursor_y)) = get_cursor_position() {
                        draw_cursor(&mut rgb_data, width, height, cursor_x, cursor_y);
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
                        display_index: current_display_index,
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

        log::info!("Multi-monitor screen capture finished. Total frames: {}", frame_count);
        Ok(())
    }
}

// Store last known cursor position in a static variable
static LAST_CURSOR_POS: std::sync::RwLock<(i32, i32)> = std::sync::RwLock::new((0, 0));

fn get_cursor_position() -> Option<(i32, i32)> {
    LAST_CURSOR_POS.read().ok().map(|pos| *pos)
}

pub fn update_cursor_position(x: i32, y: i32) {
    if let Ok(mut pos) = LAST_CURSOR_POS.write() {
        *pos = (x, y);
    }
}

fn draw_cursor(rgb_data: &mut [u8], width: usize, height: usize, cursor_x: i32, cursor_y: i32) {
    // Draw a macOS-style cursor (19x25 pixels)
    // This is a pre-defined pixel art cursor that looks like the macOS pointer

    const CURSOR_WIDTH: i32 = 19;
    const CURSOR_HEIGHT: i32 = 25;

    // Cursor pixel data (0 = transparent, 1 = black border, 2 = white fill, 3 = shadow)
    const CURSOR_PIXELS: &[&[u8]] = &[
        &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
        &[1, 2, 2, 2, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 2, 1, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 2, 1, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 1, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[1, 0, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 1, 0, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 1, 0, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    ];

    for dy in 0..CURSOR_HEIGHT {
        for dx in 0..CURSOR_WIDTH {
            let x = cursor_x + dx;
            let y = cursor_y + dy;

            if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                continue;
            }

            let pixel = CURSOR_PIXELS[dy as usize][dx as usize];
            if pixel == 0 {
                continue; // Transparent
            }

            let idx = ((y as usize) * width + (x as usize)) * 3;
            if idx + 2 >= rgb_data.len() {
                continue;
            }

            match pixel {
                1 => {
                    // Black border
                    rgb_data[idx] = 0;
                    rgb_data[idx + 1] = 0;
                    rgb_data[idx + 2] = 0;
                }
                2 => {
                    // White fill
                    rgb_data[idx] = 255;
                    rgb_data[idx + 1] = 255;
                    rgb_data[idx + 2] = 255;
                }
                _ => {}
            }
        }
    }
}
