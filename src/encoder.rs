#[cfg(target_os = "macos")]
use crate::audio::AudioSample;
use crate::capture::Frame;
use crate::db::Database;
use crate::error::{Result, ScreenRecError};
use ffmpeg_next as ffmpeg;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

#[allow(dead_code)]
pub struct RecordingOutput {
    pub video_file: PathBuf,
}

pub struct FrameMetadata {
    pub is_keyframe: bool,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub display_index: usize,
    pub width: usize,
    pub height: usize,
}

/// Encoder type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum EncoderType {
    HardwareGpu,
    HardwareCpu,  // QuickSync on CPU
    Software,
}

/// Encoder backend identification
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderInfo {
    pub name: String,
    pub encoder_type: EncoderType,
    pub priority: u8,  // Lower is higher priority
}

pub struct VideoEncoder {
    output_path: PathBuf,
    encoder: ffmpeg::encoder::Video,
    octx: ffmpeg::format::context::Output,
    stream_index: usize,
    frame_count: u64,
    pts_offset: i64, // Starting PTS for continuous timeline across chunks
    width: usize,
    height: usize,
    fps: u32,
    last_packet_keyframe: bool,
    last_packet_pts: Option<i64>,
    last_packet_dts: Option<i64>,
    encoder_info: EncoderInfo,  // Track which encoder is being used
}

/// Get platform-specific encoder priority list (GPU first)
fn get_encoder_priority_list() -> Vec<EncoderInfo> {
    #[cfg(target_os = "windows")]
    {
        vec![
            EncoderInfo { name: "h264_nvenc".to_string(), encoder_type: EncoderType::HardwareGpu, priority: 0 },
            EncoderInfo { name: "h264_qsv".to_string(), encoder_type: EncoderType::HardwareCpu, priority: 1 },
            EncoderInfo { name: "h264_amf".to_string(), encoder_type: EncoderType::HardwareGpu, priority: 2 },
            EncoderInfo { name: "libx264".to_string(), encoder_type: EncoderType::Software, priority: 10 },
            EncoderInfo { name: "h264".to_string(), encoder_type: EncoderType::Software, priority: 11 },
        ]
    }

    #[cfg(target_os = "macos")]
    {
        vec![
            EncoderInfo { name: "h264_videotoolbox".to_string(), encoder_type: EncoderType::HardwareGpu, priority: 0 },
            EncoderInfo { name: "libx264".to_string(), encoder_type: EncoderType::Software, priority: 10 },
            EncoderInfo { name: "h264".to_string(), encoder_type: EncoderType::Software, priority: 11 },
        ]
    }

    #[cfg(target_os = "linux")]
    {
        vec![
            EncoderInfo { name: "h264_vaapi".to_string(), encoder_type: EncoderType::HardwareGpu, priority: 0 },
            EncoderInfo { name: "h264_nvenc".to_string(), encoder_type: EncoderType::HardwareGpu, priority: 1 },
            EncoderInfo { name: "libx264".to_string(), encoder_type: EncoderType::Software, priority: 10 },
            EncoderInfo { name: "h264".to_string(), encoder_type: EncoderType::Software, priority: 11 },
        ]
    }
}

/// Get available encoders sorted by priority
fn get_available_encoders() -> Vec<EncoderInfo> {
    let priority_list = get_encoder_priority_list();
    let mut available = Vec::new();

    for encoder_info in priority_list {
        if ffmpeg::encoder::find_by_name(&encoder_info.name).is_some() {
            log::debug!("Encoder '{}' is available", encoder_info.name);
            available.push(encoder_info);
        } else {
            log::debug!("Encoder '{}' not available", encoder_info.name);
        }
    }

    log::info!("Available encoders: {:?}", available.iter().map(|e| &e.name).collect::<Vec<_>>());
    available
}

/// Retry configuration for encoder initialization
struct RetryConfig {
    max_retries: u32,
    initial_delay_ms: u64,
    backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 100,
            backoff_multiplier: 3.0,  // 100ms, 300ms, 900ms
        }
    }
}

impl RetryConfig {
    fn get_delay_ms(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return 0;
        }
        let delay = self.initial_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32 - 1);
        delay as u64
    }
}

/// Configure encoder-specific options
fn configure_encoder_options(
    encoder_name: &str,
    quality: u8,
    fps: u32,
    opts: &mut ffmpeg::Dictionary,
) {
    match encoder_name {
        "libx264" => {
            let crf = VideoEncoder::quality_to_crf(quality);
            opts.set("crf", &crf.to_string());
            opts.set("preset", "slow");
            opts.set("profile", "high");
            let gop_size = (fps * 2).to_string();
            opts.set("g", &gop_size);
            opts.set("keyint_min", &gop_size);
            opts.set("bf", "0");
            opts.set("refs", "3");
            opts.set("sc_threshold", "0");
            opts.set("qmin", "10");
            opts.set("qmax", "25");
            opts.set("crf_max", "18");
            opts.set("movflags", "+faststart");
        }
        "h264_videotoolbox" => {
            let crf = VideoEncoder::quality_to_crf(quality);
            opts.set("q:v", &crf.to_string());
            opts.set("profile", "high");
            opts.set("allow_sw", "1");
            let gop_size = (fps * 2).to_string();
            opts.set("g", &gop_size);
        }
        "h264_nvenc" => {
            let crf = VideoEncoder::quality_to_crf(quality);
            opts.set("cq", &crf.to_string());
            opts.set("preset", "p4");
            opts.set("tune", "hq");
            opts.set("profile", "high");
            let gop_size = (fps * 2).to_string();
            opts.set("g", &gop_size);
            opts.set("bf", "0");
        }
        "h264_qsv" => {
            let crf = VideoEncoder::quality_to_crf(quality);
            opts.set("global_quality", &crf.to_string());
            opts.set("preset", "medium");
            let gop_size = (fps * 2).to_string();
            opts.set("g", &gop_size);
        }
        "h264_amf" => {
            let crf = VideoEncoder::quality_to_crf(quality);
            opts.set("qp_i", &crf.to_string());
            opts.set("qp_p", &crf.to_string());
            opts.set("quality", "quality");
            opts.set("profile", "high");
            let gop_size = (fps * 2).to_string();
            opts.set("gops_per_idr", "1");
            opts.set("keyint_min", &gop_size);
        }
        "h264_vaapi" => {
            let crf = VideoEncoder::quality_to_crf(quality);
            opts.set("qp", &crf.to_string());
            opts.set("quality", "1");
            let gop_size = (fps * 2).to_string();
            opts.set("g", &gop_size);
        }
        "h264_mf" => {
            // Windows Media Foundation encoder
            opts.set("rate_control", "quality");
            let mf_quality = ((quality as f32 / 10.0) * 100.0).min(100.0) as i32;
            opts.set("quality", &mf_quality.to_string());
            opts.set("low_latency", "1");
            let gop_size = fps.to_string();
            opts.set("g", &gop_size);
        }
        _ => {
            // Generic fallback
            let crf = VideoEncoder::quality_to_crf(quality);
            opts.set("crf", &crf.to_string());
            opts.set("preset", "medium");
        }
    }
}

/// Single attempt to initialize encoder (no retries)
fn try_init_encoder_once(
    encoder_name: &str,
    width: u32,
    height: u32,
    fps: u32,
    quality: u8,
) -> Result<ffmpeg::encoder::Video> {
    // Find encoder
    let codec = ffmpeg::encoder::find_by_name(encoder_name)
        .ok_or_else(|| ScreenRecError::HardwareEncoderUnavailable(
            format!("Encoder '{}' not found", encoder_name)
        ))?;

    // Create encoder context
    let encoder_ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
    let mut video_encoder = encoder_ctx.encoder().video()
        .map_err(|e| ScreenRecError::EncodingError(format!("Failed to get video encoder: {}", e)))?;

    // Configure encoder
    video_encoder.set_width(width);
    video_encoder.set_height(height);
    video_encoder.set_format(ffmpeg::format::Pixel::YUV420P);
    video_encoder.set_time_base(ffmpeg::Rational::new(1, fps as i32));
    video_encoder.set_frame_rate(Some(ffmpeg::Rational::new(fps as i32, 1)));

    // Set encoder-specific options
    let mut opts = ffmpeg::Dictionary::new();
    configure_encoder_options(encoder_name, quality, fps, &mut opts);

    // Open encoder - this is where resource conflicts occur
    let encoder = video_encoder.open_with(opts)
        .map_err(|e| {
            let error_str = format!("{}", e);
            if error_str.contains("busy") || error_str.contains("in use") {
                ScreenRecError::EncoderBusy(format!("Encoder '{}' is busy: {}", encoder_name, e))
            } else {
                ScreenRecError::EncodingError(format!("Failed to open encoder '{}': {}", encoder_name, e))
            }
        })?;

    Ok(encoder)
}

/// Try to initialize an encoder with retries
fn try_init_encoder_with_retries(
    encoder_name: &str,
    width: u32,
    height: u32,
    fps: u32,
    quality: u8,
    retry_config: &RetryConfig,
) -> Result<(ffmpeg::encoder::Video, EncoderInfo)> {
    let encoder_info = get_encoder_priority_list()
        .into_iter()
        .find(|e| e.name == encoder_name)
        .unwrap_or_else(|| EncoderInfo {
            name: encoder_name.to_string(),
            encoder_type: EncoderType::Software,
            priority: 255,
        });

    let mut _last_error = None;

    for attempt in 0..=retry_config.max_retries {
        if attempt > 0 {
            let delay_ms = retry_config.get_delay_ms(attempt);
            log::info!("Retry attempt {} for encoder '{}' after {}ms delay",
                      attempt, encoder_name, delay_ms);
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }

        match try_init_encoder_once(encoder_name, width, height, fps, quality) {
            Ok(encoder) => {
                if attempt > 0 {
                    log::info!("Encoder '{}' initialized successfully on retry {}", encoder_name, attempt);
                }
                return Ok((encoder, encoder_info));
            }
            Err(e) => {
                log::warn!("Encoder '{}' initialization failed (attempt {}): {}",
                          encoder_name, attempt + 1, e);
                _last_error = Some(e);

                // Smart retry decisions based on error type
                if let Some(ref err) = _last_error {
                    let error_str = format!("{:?}", err);
                    if error_str.contains("not found") || error_str.contains("No such") {
                        break; // Don't retry if encoder doesn't exist
                    }
                }
            }
        }
    }

    Err(ScreenRecError::EncoderInitializationFailed(
        format!("Failed to initialize '{}' after {} retries", encoder_name, retry_config.max_retries),
        retry_config.max_retries,
    ))
}

impl VideoEncoder {
    pub fn new_with_pts_offset<F>(
        output_path: &Path,
        width: usize,
        height: usize,
        fps: u32,
        quality: u8,
        pts_offset: i64,
        on_chunk_created: Option<F>,
    ) -> Result<Self>
    where
        F: FnOnce(&str),
    {
        // Initialize FFmpeg
        ffmpeg::init().map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to initialize FFmpeg: {}", e))
        })?;

        log::info!(
            "Initializing MP4 encoder: {}x{} @ {}fps",
            width,
            height,
            fps
        );

        // Determine output path
        let output_path = if output_path.extension().is_some() {
            output_path.to_path_buf()
        } else {
            let mut path = output_path.to_path_buf();
            path.set_extension("mp4");
            path
        };

        // Create output context
        let mut octx = ffmpeg::format::output(&output_path).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to create output context: {}", e))
        })?;

        // GPU-first encoder selection with retry logic and fallback chain
        log::info!("Initializing encoder with GPU-first priority");

        let available_encoders = get_available_encoders();
        if available_encoders.is_empty() {
            return Err(ScreenRecError::EncodingError(
                "No H.264 encoders available on this system".to_string()
            ));
        }

        let retry_config = RetryConfig::default();
        let mut tried_encoders = Vec::new();

        // Try each encoder in priority order with retries
        let mut encoder_result = None;
        let mut selected_encoder_info = None;

        for encoder_info in &available_encoders {
            log::info!("Attempting encoder: {} (type: {:?}, priority: {})",
                      encoder_info.name, encoder_info.encoder_type, encoder_info.priority);
            tried_encoders.push(encoder_info.name.clone());

            match try_init_encoder_with_retries(
                &encoder_info.name,
                width as u32,
                height as u32,
                fps,
                quality,
                &retry_config,
            ) {
                Ok((encoder, info)) => {
                    log::info!("✓ Successfully initialized encoder: {} ({:?})",
                              info.name, info.encoder_type);
                    encoder_result = Some(encoder);
                    selected_encoder_info = Some(info);
                    break;
                }
                Err(e) => {
                    log::warn!("Failed to initialize encoder '{}': {}", encoder_info.name, e);
                    // Continue to next encoder in fallback chain
                }
            }
        }

        let encoder = encoder_result.ok_or_else(|| {
            ScreenRecError::EncodingError(format!(
                "All encoders failed. Tried: {:?}", tried_encoders
            ))
        })?;

        let encoder_info = selected_encoder_info.unwrap();
        let encoder_name = encoder_info.name.clone();

        // Get codec for stream setup
        let codec = ffmpeg::encoder::find_by_name(&encoder_name)
            .ok_or_else(|| ScreenRecError::EncodingError("Selected encoder codec not found".to_string()))?;

        log::info!("Using encoder: {} (type: {:?})", encoder_name, encoder_info.encoder_type);

        // Create video stream
        let mut stream = octx.add_stream(codec).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to add stream: {}", e))
        })?;
        let stream_index = stream.index();

        // Copy encoder parameters to stream first
        stream.set_parameters(&encoder);

        // Then set time_base (1/90000 is MP4 standard) and frame rate AFTER parameters
        stream.set_time_base(ffmpeg::Rational(1, 90000));
        stream.set_avg_frame_rate(ffmpeg::Rational(fps as i32, 1));

        // Write header
        octx.write_header().map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to write header: {}", e))
        })?;

        log::info!("MP4 encoder initialized: {}", output_path.display());

        // Call callback if provided
        if let Some(callback) = on_chunk_created {
            callback(output_path.to_str().unwrap_or("unknown"));
        }

        log::info!("Encoder initialized with PTS offset: {}", pts_offset);

        Ok(Self {
            output_path,
            encoder,
            octx,
            stream_index,
            frame_count: 0,
            pts_offset,
            width,
            height,
            fps,
            last_packet_keyframe: false,
            last_packet_pts: None,
            last_packet_dts: None,
            encoder_info,
        })
    }

    /// Attempt to recover from encoder failure by switching to fallback encoder
    fn try_recover_encoder(&mut self, error: &ScreenRecError) -> Result<()> {
        log::error!("Encoder failure detected: {}. Attempting recovery...", error);

        let available_encoders = get_available_encoders();
        let current_priority = self.encoder_info.priority;

        // Find next encoder with lower priority (higher number)
        let fallback_encoder = available_encoders.iter()
            .find(|e| e.priority > current_priority)
            .cloned();

        match fallback_encoder {
            Some(fallback_info) => {
                log::info!("Attempting fallback to encoder: {} ({:?})",
                          fallback_info.name, fallback_info.encoder_type);

                // Single attempt, no retries during recording
                match try_init_encoder_once(
                    &fallback_info.name,
                    self.width as u32,
                    self.height as u32,
                    self.fps,
                    8, // Default quality for recovery
                ) {
                    Ok(new_encoder) => {
                        log::info!("✓ Successfully switched to fallback encoder: {}", fallback_info.name);

                        // Hot-swap the encoder
                        self.encoder = new_encoder;
                        self.encoder_info = fallback_info;

                        Ok(())
                    }
                    Err(e) => {
                        log::error!("Failed to initialize fallback encoder: {}", e);
                        Err(ScreenRecError::EncoderRuntimeFailure(
                            format!("Failed to recover encoder: {}", e)
                        ))
                    }
                }
            }
            None => {
                log::error!("No fallback encoder available for recovery");
                Err(ScreenRecError::EncoderRuntimeFailure(
                    "No fallback encoder available".to_string()
                ))
            }
        }
    }

    pub fn encode_frame(&mut self, frame: Frame) -> Result<FrameMetadata> {
        let Frame { data, width, height, display_index, .. } = frame;

        // If frame dimensions don't match encoder dimensions, we need to scale/pad
        let processed_data = if width != self.width || height != self.height {
            log::debug!("Frame dimensions {}x{} don't match encoder {}x{}, scaling/padding",
                       width, height, self.width, self.height);
            Self::scale_and_pad_frame(&data, width, height, self.width, self.height)?
        } else {
            data
        };

        // Create YUV420P frame
        let mut yuv_frame = ffmpeg::frame::Video::new(
            ffmpeg::format::Pixel::YUV420P,
            self.width as u32,
            self.height as u32,
        );

        // Set PTS in encoder time_base units (1/fps)
        // Each chunk's PTS should start from 0 for the file itself
        // pts_offset is only used for tracking logical frame numbers
        let pts = self.frame_count as i64;
        yuv_frame.set_pts(Some(pts));

        // Convert RGB to YUV420P
        Self::rgb_to_yuv420p(&processed_data, self.width, self.height, &mut yuv_frame)?;

        // Send frame to encoder with recovery on failure
        match self.encoder.send_frame(&yuv_frame) {
            Ok(_) => {
                // Success, continue
            }
            Err(e) => {
                let error = ScreenRecError::EncoderRuntimeFailure(
                    format!("Failed to send frame: {}", e)
                );
                log::warn!("Frame encoding failed, attempting recovery...");

                // Try to recover with fallback encoder (one attempt only)
                self.try_recover_encoder(&error)?;

                // Retry frame with new encoder
                self.encoder.send_frame(&yuv_frame).map_err(|e| {
                    ScreenRecError::EncoderRuntimeFailure(
                        format!("Failed to send frame after recovery: {}", e)
                    )
                })?;
            }
        }

        // Receive and write packets (this updates last_packet_* fields)
        self.receive_packets()?;

        // Create metadata from the last encoded packet
        let metadata = FrameMetadata {
            is_keyframe: self.last_packet_keyframe,
            pts: self.last_packet_pts,
            dts: self.last_packet_dts,
            display_index,
            width,
            height,
        };

        self.frame_count += 1;

        if self.frame_count % (self.fps as u64) == 0 {
            log::debug!("Encoded {} frames", self.frame_count);
        }

        Ok(metadata)
    }

    fn receive_packets(&mut self) -> Result<()> {
        let mut encoded = ffmpeg::Packet::empty();
        while self.encoder.receive_packet(&mut encoded).is_ok() {
            // Capture packet metadata before rescaling
            self.last_packet_keyframe = encoded.is_key();
            self.last_packet_pts = encoded.pts();
            self.last_packet_dts = encoded.dts();

            encoded.set_stream(self.stream_index);

            // Rescale timestamps from encoder time_base (1/fps) to stream time_base (1/90000)
            encoded.rescale_ts(
                ffmpeg::Rational(1, self.fps as i32), // from encoder time_base (1/fps)
                ffmpeg::Rational(1, 90000),            // to stream time_base (1/90000)
            );

            encoded
                .write_interleaved(&mut self.octx)
                .map_err(|e| {
                    ScreenRecError::EncodingError(format!("Failed to write packet: {}", e))
                })?;
        }
        Ok(())
    }

    /// Get the next logical frame number that should be used for the next chunk
    /// This is for tracking purposes, not for PTS (each chunk starts PTS at 0)
    pub fn get_next_pts(&self) -> i64 {
        self.pts_offset + (self.frame_count as i64)
    }

    pub fn finish(mut self) -> Result<RecordingOutput> {
        log::info!("Finishing encoding, total frames: {}", self.frame_count);

        // Flush encoder
        self.encoder.send_eof().map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to send EOF: {}", e))
        })?;

        self.receive_packets()?;

        // Write trailer
        self.octx.write_trailer().map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to write trailer: {}", e))
        })?;

        log::info!("Video saved to: {}", self.output_path.display());

        Ok(RecordingOutput {
            video_file: self.output_path,
        })
    }

    fn rgb_to_yuv420p(
        rgb: &[u8],
        width: usize,
        height: usize,
        yuv_frame: &mut ffmpeg::frame::Video,
    ) -> Result<()> {
        // Get strides
        let y_stride = yuv_frame.stride(0);
        let u_stride = yuv_frame.stride(1);

        // Use integer arithmetic for much faster conversion
        unsafe {
            let y_ptr = yuv_frame.data_mut(0).as_mut_ptr();
            let u_ptr = yuv_frame.data_mut(1).as_mut_ptr();
            let v_ptr = yuv_frame.data_mut(2).as_mut_ptr();

            // Process Y plane (all pixels)
            for y in 0..height {
                let y_row_offset = y * y_stride;
                let rgb_row_offset = y * width * 3;

                for x in 0..width {
                    let rgb_idx = rgb_row_offset + x * 3;
                    let r = rgb[rgb_idx] as i32;
                    let g = rgb[rgb_idx + 1] as i32;
                    let b = rgb[rgb_idx + 2] as i32;

                    // Y = 0.299*R + 0.587*G + 0.114*B (using fixed-point arithmetic)
                    let y_val = ((77 * r + 150 * g + 29 * b) >> 8) as u8;
                    *y_ptr.add(y_row_offset + x) = y_val;
                }
            }

            // Process U and V planes (2x2 subsampling)
            for y in (0..height).step_by(2) {
                let uv_y = y / 2;
                let uv_row_offset = uv_y * u_stride;
                let rgb_row_offset = y * width * 3;

                for x in (0..width).step_by(2) {
                    let rgb_idx = rgb_row_offset + x * 3;
                    let r = rgb[rgb_idx] as i32;
                    let g = rgb[rgb_idx + 1] as i32;
                    let b = rgb[rgb_idx + 2] as i32;

                    let uv_x = x / 2;

                    // U = -0.169*R - 0.331*G + 0.500*B + 128
                    let u_val = (((-43 * r - 85 * g + 128 * b) >> 8) + 128).clamp(0, 255) as u8;
                    *u_ptr.add(uv_row_offset + uv_x) = u_val;

                    // V = 0.500*R - 0.419*G - 0.081*B + 128
                    let v_val = (((128 * r - 107 * g - 21 * b) >> 8) + 128).clamp(0, 255) as u8;
                    *v_ptr.add(uv_row_offset + uv_x) = v_val;
                }
            }
        }

        Ok(())
    }

    /// Scale and pad frame to target dimensions (center with black bars)
    fn scale_and_pad_frame(
        rgb: &[u8],
        src_width: usize,
        src_height: usize,
        target_width: usize,
        target_height: usize,
    ) -> Result<Vec<u8>> {
        // Calculate scaling to fit within target while preserving aspect ratio
        let width_ratio = target_width as f32 / src_width as f32;
        let height_ratio = target_height as f32 / src_height as f32;
        let scale_ratio = width_ratio.min(height_ratio);

        let scaled_width = (src_width as f32 * scale_ratio) as usize;
        let scaled_height = (src_height as f32 * scale_ratio) as usize;

        // Calculate padding to center the scaled image
        let pad_x = (target_width - scaled_width) / 2;
        let pad_y = (target_height - scaled_height) / 2;

        // Create black canvas
        let mut result = vec![0u8; target_width * target_height * 3];

        // Simple nearest-neighbor scaling and placement
        for target_y in 0..target_height {
            for target_x in 0..target_width {
                // Check if we're in the scaled image area
                if target_x >= pad_x && target_x < pad_x + scaled_width &&
                   target_y >= pad_y && target_y < pad_y + scaled_height {
                    // Map to source coordinates
                    let src_x = ((target_x - pad_x) as f32 / scale_ratio) as usize;
                    let src_y = ((target_y - pad_y) as f32 / scale_ratio) as usize;

                    // Bounds check
                    if src_x < src_width && src_y < src_height {
                        let src_idx = (src_y * src_width + src_x) * 3;
                        let dst_idx = (target_y * target_width + target_x) * 3;

                        if src_idx + 2 < rgb.len() && dst_idx + 2 < result.len() {
                            result[dst_idx] = rgb[src_idx];         // R
                            result[dst_idx + 1] = rgb[src_idx + 1]; // G
                            result[dst_idx + 2] = rgb[src_idx + 2]; // B
                        }
                    }
                }
                // Else: leave as black (already initialized to 0)
            }
        }

        Ok(result)
    }

    fn quality_to_crf(quality: u8) -> u8 {
        #[cfg(target_os = "windows")]
        {
            // For Windows screen recording, use very low CRF values for crystal-clear text and UI
            // Quality 1 (lowest) -> CRF 28 (acceptable for low-detail content)
            // Quality 5 (medium) -> CRF 18 (good balance)
            // Quality 8 (high)   -> CRF 12 (excellent quality)
            // Quality 10 (max)   -> CRF 8  (near-lossless for screen content)
            let q = quality.clamp(1, 10) as i32;
            // Use quadratic mapping for better quality at high settings
            // Formula: 30 - (q * 2.2) to get more aggressive at higher quality
            let mapped = 30 - ((q * 22) / 10); // More aggressive: q=8 -> CRF 12, q=10 -> CRF 8
            mapped.clamp(8, 28) as u8
        }

        #[cfg(not(target_os = "windows"))]
        {
            // Mac/Linux: use original mapping
            let q = quality.clamp(1, 10) as i32;
            let mapped = 42 - q * 3;
            mapped.clamp(12, 35) as u8
        }
    }
}

/// Process frames from the capture channel and encode them
#[allow(dead_code)]
pub async fn process_frames(
    mut rx: mpsc::Receiver<Frame>,
    mut encoder: VideoEncoder,
    db: Option<Arc<Database>>,
    device_name: Option<String>,
) -> Result<RecordingOutput> {
    log::info!("Starting frame processing");

    while let Some(frame) = rx.recv().await {
        let captured_at = frame.captured_at;

        // Encode frame and get metadata
        let metadata = encoder.encode_frame(frame)?;

        // Insert frame into database with metadata if enabled
        if let (Some(ref db), Some(ref device)) = (&db, &device_name) {
            if let Err(e) = db
                .insert_frame(
                    device,
                    Some(captured_at),
                    metadata.is_keyframe,
                    metadata.pts,
                    metadata.dts,
                    Some(metadata.display_index as i64),
                    Some(metadata.width as i64),
                    Some(metadata.height as i64),
                )
                .await
            {
                log::error!("Failed to insert frame into database: {}", e);
            }
        }
    }

    encoder.finish()
}

/// Process frames with chunking support
pub async fn process_frames_chunked(
    mut rx: mpsc::Receiver<Frame>,
    base_output_dir: PathBuf,
    width: usize,
    height: usize,
    fps: u32,
    quality: u8,
    chunk_duration_secs: u64,
    db: Option<Arc<Database>>,
    device_name: Option<String>,
    recording_type: Option<String>,
    task_id: Option<String>,
    session_id: Option<i64>,
    mut shutdown_rx: Option<tokio::sync::oneshot::Receiver<()>>,
) -> Result<Vec<RecordingOutput>> {
    log::info!("Starting chunked frame processing with {}-second chunks", chunk_duration_secs);

    let mut chunk_outputs = Vec::new();
    let mut chunk_index = 0i64;
    let frames_per_chunk = (fps as u64) * chunk_duration_secs;
    let mut frames_in_current_chunk = 0u64;
    let mut next_pts_offset = 0i64; // Track continuous PTS across chunks
    let mut total_frames_encoded = 0u64;

    // Create first chunk
    let now = chrono::Local::now();
    let chunk_filename = format!("{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
    let chunk_path = base_output_dir.join(&chunk_filename);

    log::info!("Creating chunk {}: {} (PTS offset: {})", chunk_index, chunk_path.display(), next_pts_offset);

    let mut current_encoder = VideoEncoder::new_with_pts_offset(
        &chunk_path,
        width,
        height,
        fps,
        quality,
        next_pts_offset,
        None::<fn(&str)>,
    )?;

    // Insert video chunk into database
    if let (Some(ref db), Some(ref device)) = (&db, &device_name) {
        if let Err(e) = db.insert_video_chunk(
            chunk_path.to_str().unwrap_or(""),
            device,
            recording_type.as_deref(),
            task_id.as_deref(),
            Some(chunk_index),
            session_id,
            Some(fps as i64),
        ).await {
            log::error!("Failed to insert video chunk into database: {}", e);
        }
    }

    loop {
        // Wait for either a frame or shutdown signal
        let should_shutdown = if let Some(ref mut shutdown) = shutdown_rx {
            tokio::select! {
                frame_opt = rx.recv() => {
                    if let Some(frame) = frame_opt {
                        let captured_at = frame.captured_at;

                        // Check if we need to start a new chunk
                        if frames_in_current_chunk >= frames_per_chunk {
                            log::info!("Finishing chunk {} with {} frames", chunk_index, frames_in_current_chunk);

                            // Get next PTS before finishing encoder
                            next_pts_offset = current_encoder.get_next_pts();

                            // Finish current encoder
                            let output = current_encoder.finish()?;
                            chunk_outputs.push(output);

                            // Start new chunk
                            chunk_index += 1;
                            frames_in_current_chunk = 0;

                            let now = chrono::Local::now();
                            let chunk_filename = format!("{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
                            let chunk_path = base_output_dir.join(&chunk_filename);

                            log::info!("Creating chunk {}: {} (PTS offset: {})", chunk_index, chunk_path.display(), next_pts_offset);

                            current_encoder = VideoEncoder::new_with_pts_offset(
                                &chunk_path,
                                width,
                                height,
                                fps,
                                quality,
                                next_pts_offset,
                                None::<fn(&str)>,
                            )?;

                            // Insert new video chunk into database
                            if let (Some(ref db), Some(ref device)) = (&db, &device_name) {
                                if let Err(e) = db.insert_video_chunk(
                                    chunk_path.to_str().unwrap_or(""),
                                    device,
                                    recording_type.as_deref(),
                                    task_id.as_deref(),
                                    Some(chunk_index),
                                    session_id,
                                    Some(fps as i64),
                                ).await {
                                    log::error!("Failed to insert video chunk into database: {}", e);
                                }
                            }
                        }

                        // Encode frame and get metadata
                        let metadata = current_encoder.encode_frame(frame)?;
                        frames_in_current_chunk += 1;
                        total_frames_encoded += 1;

                        // Insert frame into database with metadata if enabled
                        if let (Some(ref db), Some(ref device)) = (&db, &device_name) {
                            if let Err(e) = db
                                .insert_frame(
                                    device,
                                    Some(captured_at),
                                    metadata.is_keyframe,
                                    metadata.pts,
                                    metadata.dts,
                                    Some(metadata.display_index as i64),
                                    Some(metadata.width as i64),
                                    Some(metadata.height as i64),
                                )
                                .await
                            {
                                log::error!("Failed to insert frame into database: {}", e);
                            }
                        }
                        false
                    } else {
                        // Channel closed, finish processing
                        break;
                    }
                }
                _ = shutdown => {
                    log::warn!("Shutdown signal received, finalizing current chunk...");
                    true
                }
            }
        } else {
            // No shutdown receiver, just wait for frame
            if let Some(frame) = rx.recv().await {
                let captured_at = frame.captured_at;

                // Check if we need to start a new chunk
                if frames_in_current_chunk >= frames_per_chunk {
                    log::debug!("Starting new chunk - total frames encoded so far: {}", total_frames_encoded);
                    log::info!("Finishing chunk {} with {} frames", chunk_index, frames_in_current_chunk);

                    // Get next PTS before finishing encoder
                    next_pts_offset = current_encoder.get_next_pts();

                    // Finish current encoder
                    let output = current_encoder.finish()?;
                    chunk_outputs.push(output);

                    // Start new chunk
                    chunk_index += 1;
                    frames_in_current_chunk = 0;

                    let now = chrono::Local::now();
                    let chunk_filename = format!("{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
                    let chunk_path = base_output_dir.join(&chunk_filename);

                    log::info!("Creating chunk {}: {} (PTS offset: {})", chunk_index, chunk_path.display(), next_pts_offset);

                    current_encoder = VideoEncoder::new_with_pts_offset(
                        &chunk_path,
                        width,
                        height,
                        fps,
                        quality,
                        next_pts_offset,
                        None::<fn(&str)>,
                    )?;

                    // Insert new video chunk into database
                    if let (Some(ref db), Some(ref device)) = (&db, &device_name) {
                        if let Err(e) = db.insert_video_chunk(
                            chunk_path.to_str().unwrap_or(""),
                            device,
                            recording_type.as_deref(),
                            task_id.as_deref(),
                            Some(chunk_index),
                            session_id,
                            Some(fps as i64),
                        ).await {
                            log::error!("Failed to insert video chunk into database: {}", e);
                        }
                    }
                }

                // Encode frame and get metadata
                let metadata = current_encoder.encode_frame(frame)?;
                frames_in_current_chunk += 1;
                total_frames_encoded += 1;

                // Log every second worth of frames
                if total_frames_encoded % fps as u64 == 0 {
                    log::debug!("Encoded {} total frames ({} in current chunk)", total_frames_encoded, frames_in_current_chunk);
                }

                // Insert frame into database with metadata if enabled
                if let (Some(ref db), Some(ref device)) = (&db, &device_name) {
                    if let Err(e) = db
                        .insert_frame(
                            device,
                            Some(captured_at),
                            metadata.is_keyframe,
                            metadata.pts,
                            metadata.dts,
                            Some(metadata.display_index as i64),
                            Some(metadata.width as i64),
                            Some(metadata.height as i64),
                        )
                        .await
                    {
                        log::error!("Failed to insert frame into database: {}", e);
                    }
                }
                false
            } else {
                // Channel closed, finish processing
                break;
            }
        };

        if should_shutdown {
            log::info!("Gracefully shutting down encoder...");
            break;
        }
    }

    // Finish the last chunk
    log::info!("Finishing final chunk {} with {} frames", chunk_index, frames_in_current_chunk);
    let output = current_encoder.finish()?;
    chunk_outputs.push(output);

    log::info!("===== ENCODING COMPLETE =====");
    log::info!("Total frames encoded: {}", total_frames_encoded);
    log::info!("Chunks created: {}", chunk_outputs.len());
    log::info!("Expected frames per chunk: {}", frames_per_chunk);
    log::info!("============================");
    Ok(chunk_outputs)
}

/// Process audio samples (currently just logs them, can be extended to save audio file)
#[cfg(target_os = "macos")]
pub async fn process_audio(mut rx: mpsc::Receiver<AudioSample>) -> Result<()> {
    log::info!("Starting audio processing");

    let mut sample_count = 0u64;
    while let Some(_sample) = rx.recv().await {
        sample_count += 1;
        if sample_count % 100 == 0 {
            log::debug!("Received {} audio samples", sample_count);
        }
    }

    log::info!("Audio processing finished, total samples: {}", sample_count);
    Ok(())
}
