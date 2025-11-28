//! macOS-specific subprocess-based encoder
//!
//! This module provides a VideoEncoder implementation that uses FFmpeg as a subprocess
//! instead of linking to FFmpeg libraries. This avoids dependency issues on macOS while
//! maintaining the same API as the library-based encoder used on Windows/Linux.

#[cfg(target_os = "macos")]
use crate::capture::Frame;
use crate::encoder::{EncoderInfo, EncoderType, FrameMetadata, RecordingOutput};
use crate::error::{Result, ScreenRecError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Subprocess-based video encoder for macOS
pub struct SubprocessEncoder {
    process: Child,
    stdin: std::io::BufWriter<std::process::ChildStdin>,
    output_path: PathBuf,
    width: usize,
    height: usize,
    fps: u32,
    frame_count: u64,
    pts_offset: i64,
    gop_size: u32,
    encoder_info: EncoderInfo,
    ffmpeg_path: String,
}

impl SubprocessEncoder {
    pub fn new_with_pts_offset<F>(
        output_path: &Path,
        width: usize,
        height: usize,
        fps: u32,
        quality: u8,
        pts_offset: i64,
        on_chunk_created: Option<F>,
        ffmpeg_path: &str,
    ) -> Result<Self>
    where
        F: FnOnce(&str),
    {
        log::info!(
            "Initializing subprocess MP4 encoder: {}x{} @ {}fps",
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

        // Get available encoders
        let available_encoders = get_available_encoders(ffmpeg_path)?;
        if available_encoders.is_empty() {
            return Err(ScreenRecError::EncodingError(
                "No H.264 encoders available on this system".to_string()
            ));
        }

        // Try each encoder in priority order
        let mut last_error = None;
        for encoder_info in &available_encoders {
            log::info!(
                "Attempting encoder: {} (type: {:?}, priority: {})",
                encoder_info.name,
                encoder_info.encoder_type,
                encoder_info.priority
            );

            match spawn_ffmpeg_encoder(
                ffmpeg_path,
                &encoder_info.name,
                &output_path,
                width,
                height,
                fps,
                quality,
            ) {
                Ok((process, stdin)) => {
                    log::info!("✓ Successfully initialized encoder: {} ({:?})",
                              encoder_info.name, encoder_info.encoder_type);

                    let gop_size = fps * 2; // GOP size is 2 seconds

                    // Call callback if provided
                    if let Some(callback) = on_chunk_created {
                        callback(output_path.to_str().unwrap_or("unknown"));
                    }

                    log::info!("Encoder initialized with PTS offset: {}", pts_offset);

                    return Ok(Self {
                        process,
                        stdin,
                        output_path,
                        width,
                        height,
                        fps,
                        frame_count: 0,
                        pts_offset,
                        gop_size,
                        encoder_info: encoder_info.clone(),
                        ffmpeg_path: ffmpeg_path.to_string(),
                    });
                }
                Err(e) => {
                    log::warn!("Failed to initialize encoder '{}': {}", encoder_info.name, e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            ScreenRecError::EncodingError("All encoders failed".to_string())
        }))
    }

    pub fn encode_frame(&mut self, frame: Frame) -> Result<FrameMetadata> {
        let Frame { data, width, height, display_index, .. } = frame;

        // If frame dimensions don't match encoder dimensions, we need to scale/pad
        let processed_data = if width != self.width || height != self.height {
            log::debug!(
                "Frame dimensions {}x{} don't match encoder {}x{}, scaling/padding",
                width,
                height,
                self.width,
                self.height
            );
            scale_and_pad_frame(&data, width, height, self.width, self.height)?
        } else {
            data
        };

        // Write raw RGB24 frame data to FFmpeg stdin
        match self.stdin.write_all(&processed_data) {
            Ok(_) => {
                // Flush every second to ensure frames are processed
                if self.frame_count % (self.fps as u64) == 0 {
                    self.stdin.flush().map_err(|e| {
                        ScreenRecError::EncodingError(format!("Failed to flush stdin: {}", e))
                    })?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                // FFmpeg process died - try to get error from stderr
                return Err(ScreenRecError::EncoderRuntimeFailure(
                    format!("FFmpeg process died: {}", e)
                ));
            }
            Err(e) => {
                return Err(ScreenRecError::EncodingError(format!(
                    "Failed to write frame to stdin: {}",
                    e
                )));
            }
        }

        // Calculate metadata based on frame count
        let is_keyframe = self.frame_count % (self.gop_size as u64) == 0;
        let pts = Some(self.frame_count as i64);
        let dts = pts; // No B-frames, so DTS = PTS

        let metadata = FrameMetadata {
            is_keyframe,
            pts,
            dts,
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

    /// Get the next logical frame number that should be used for the next chunk
    pub fn get_next_pts(&self) -> i64 {
        self.pts_offset + (self.frame_count as i64)
    }

    pub fn finish(mut self) -> Result<RecordingOutput> {
        log::info!("Finishing encoding, total frames: {}", self.frame_count);

        // Flush and close stdin to signal EOF to FFmpeg
        if let Err(e) = self.stdin.flush() {
            log::warn!("Failed to flush stdin before closing: {}", e);
        }
        drop(self.stdin);

        // Wait for FFmpeg process to finish
        match self.process.wait() {
            Ok(status) => {
                if !status.success() {
                    let code = status.code().unwrap_or(-1);
                    return Err(ScreenRecError::EncodingError(format!(
                        "FFmpeg process exited with code {}",
                        code
                    )));
                }
            }
            Err(e) => {
                return Err(ScreenRecError::EncodingError(format!(
                    "Failed to wait for FFmpeg process: {}",
                    e
                )));
            }
        }

        // Verify output file exists
        if !self.output_path.exists() {
            return Err(ScreenRecError::EncodingError(format!(
                "Output file was not created: {}",
                self.output_path.display()
            )));
        }

        log::info!("Video saved to: {}", self.output_path.display());

        Ok(RecordingOutput {
            video_file: self.output_path,
        })
    }
}

/// Get platform-specific encoder priority list for macOS
fn get_encoder_priority_list() -> Vec<EncoderInfo> {
    vec![
        EncoderInfo {
            name: "h264_videotoolbox".to_string(),
            encoder_type: EncoderType::HardwareGpu,
            priority: 0,
        },
        EncoderInfo {
            name: "libx264".to_string(),
            encoder_type: EncoderType::Software,
            priority: 10,
        },
        EncoderInfo {
            name: "h264".to_string(),
            encoder_type: EncoderType::Software,
            priority: 11,
        },
    ]
}

/// Get available encoders by checking FFmpeg
fn get_available_encoders(ffmpeg_path: &str) -> Result<Vec<EncoderInfo>> {
    log::info!("Detecting available encoders...");

    let output = Command::new(ffmpeg_path)
        .arg("-encoders")
        .output()
        .map_err(|e| {
            ScreenRecError::ConfigError(format!("Failed to run ffmpeg -encoders: {}", e))
        })?;

    if !output.status.success() {
        return Err(ScreenRecError::ConfigError(
            "Failed to get encoder list from FFmpeg".to_string()
        ));
    }

    let output_str = String::from_utf8_lossy(&output.stdout);
    let priority_list = get_encoder_priority_list();
    let mut available = Vec::new();

    for encoder_info in priority_list {
        // Check if encoder is in the output
        if output_str.contains(&format!(" {} ", encoder_info.name))
            || output_str.contains(&format!("{}  ", encoder_info.name)) {
            log::debug!("Encoder '{}' is available", encoder_info.name);
            available.push(encoder_info);
        } else {
            log::debug!("Encoder '{}' not available", encoder_info.name);
        }
    }

    log::info!(
        "Available encoders: {:?}",
        available.iter().map(|e| &e.name).collect::<Vec<_>>()
    );

    Ok(available)
}

/// Spawn FFmpeg process for encoding
fn spawn_ffmpeg_encoder(
    ffmpeg_path: &str,
    encoder_name: &str,
    output_path: &Path,
    width: usize,
    height: usize,
    fps: u32,
    quality: u8,
) -> Result<(Child, std::io::BufWriter<std::process::ChildStdin>)> {
    let gop_size = fps * 2;

    // Build encoder-specific arguments
    let mut args = vec![
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pixel_format".to_string(),
        "rgb24".to_string(),
        "-video_size".to_string(),
        format!("{}x{}", width, height),
        "-framerate".to_string(),
        fps.to_string(),
        "-i".to_string(),
        "pipe:0".to_string(),
        "-c:v".to_string(),
        encoder_name.to_string(),
        "-fps_mode".to_string(),
        "cfr".to_string(), // Force constant frame rate - duplicate/drop frames as needed
        "-r".to_string(),
        fps.to_string(), // Output frame rate
    ];

    // Add encoder-specific quality parameters
    match encoder_name {
        "h264_videotoolbox" => {
            let bitrate = quality_to_bitrate(quality, width, height, fps);
            args.extend_from_slice(&[
                "-b:v".to_string(),
                bitrate,
                "-profile:v".to_string(),
                "high".to_string(),
                "-allow_sw".to_string(),
                "1".to_string(),
                "-g".to_string(),
                gop_size.to_string(),
            ]);
        }
        "libx264" => {
            let crf = quality_to_crf(quality);
            args.extend_from_slice(&[
                "-crf".to_string(),
                crf.to_string(),
                "-preset".to_string(),
                "slow".to_string(),
                "-profile:v".to_string(),
                "high".to_string(),
                "-g".to_string(),
                gop_size.to_string(),
            ]);
        }
        _ => {
            // Generic fallback
            let crf = quality_to_crf(quality);
            args.extend_from_slice(&[
                "-crf".to_string(),
                crf.to_string(),
                "-preset".to_string(),
                "medium".to_string(),
                "-g".to_string(),
                gop_size.to_string(),
            ]);
        }
    }

    // Add output format parameters
    args.extend_from_slice(&[
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-movflags".to_string(),
        "frag_keyframe+empty_moov".to_string(),
        "-f".to_string(),
        "mp4".to_string(),
        output_path.to_str().unwrap().to_string(),
    ]);

    log::debug!("Spawning FFmpeg with args: {:?}", args);

    let mut child = Command::new(ffmpeg_path)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit()) // Show FFmpeg errors in console
        .spawn()
        .map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to spawn FFmpeg process: {}", e))
        })?;

    let stdin = child.stdin.take().ok_or_else(|| {
        ScreenRecError::EncodingError("Failed to get FFmpeg stdin".to_string())
    })?;

    let buffered_stdin = std::io::BufWriter::with_capacity(1024 * 1024, stdin); // 1MB buffer

    Ok((child, buffered_stdin))
}

/// Convert quality (1-10) to bitrate for VideoToolbox
/// Quality 1 = 2 Mbps, Quality 10 = 20 Mbps (scales with resolution)
fn quality_to_bitrate(quality: u8, width: usize, height: usize, fps: u32) -> String {
    let quality = quality.clamp(1, 10) as f32;

    // Base bitrate calculation: pixels * fps * bits_per_pixel
    // Quality affects bits_per_pixel: 1=0.08, 10=0.30
    let bits_per_pixel = 0.08 + (quality - 1.0) * 0.025; // 0.08 to 0.305
    let pixels = (width * height) as f32;
    let bitrate_bps = pixels * fps as f32 * bits_per_pixel;
    let bitrate_mbps = bitrate_bps / 1_000_000.0;

    format!("{}M", bitrate_mbps as u32)
}

/// Convert quality (1-10) to CRF value
fn quality_to_crf(quality: u8) -> u8 {
    let q = quality.clamp(1, 10) as i32;
    let mapped = 42 - q * 3;
    mapped.clamp(12, 35) as u8
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
            if target_x >= pad_x
                && target_x < pad_x + scaled_width
                && target_y >= pad_y
                && target_y < pad_y + scaled_height
            {
                // Map to source coordinates
                let src_x = ((target_x - pad_x) as f32 / scale_ratio) as usize;
                let src_y = ((target_y - pad_y) as f32 / scale_ratio) as usize;

                // Bounds check
                if src_x < src_width && src_y < src_height {
                    let src_idx = (src_y * src_width + src_x) * 3;
                    let dst_idx = (target_y * target_width + target_x) * 3;

                    if src_idx + 2 < rgb.len() && dst_idx + 2 < result.len() {
                        result[dst_idx] = rgb[src_idx]; // R
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
