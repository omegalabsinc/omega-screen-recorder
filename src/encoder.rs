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

pub struct VideoEncoder {
    output_path: PathBuf,
    encoder: ffmpeg::encoder::Video,
    octx: ffmpeg::format::context::Output,
    stream_index: usize,
    frame_count: u64,
    width: usize,
    height: usize,
    fps: u32,
    last_packet_keyframe: bool,
    last_packet_pts: Option<i64>,
    last_packet_dts: Option<i64>,
}

impl VideoEncoder {
    pub fn new<F>(
        output_path: &Path,
        width: usize,
        height: usize,
        fps: u32,
        quality: u8,
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

        // Find H.264 encoder
        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::H264)
            .ok_or_else(|| ScreenRecError::EncodingError("H.264 codec not found".to_string()))?;

        // Create encoder context from codec
        let encoder_ctx = ffmpeg::codec::context::Context::new_with_codec(codec);
        let mut video_encoder = encoder_ctx.encoder().video().map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to get video encoder: {}", e))
        })?;

        // Configure encoder
        video_encoder.set_width(width as u32);
        video_encoder.set_height(height as u32);
        video_encoder.set_format(ffmpeg::format::Pixel::YUV420P);
        video_encoder.set_time_base(ffmpeg::Rational::new(1, fps as i32));
        video_encoder.set_frame_rate(Some(ffmpeg::Rational::new(fps as i32, 1)));

        // Set quality (CRF)
        let crf = Self::quality_to_crf(quality);
        let mut opts = ffmpeg::Dictionary::new();
        opts.set("crf", &crf.to_string());
        opts.set("preset", "medium");

        // Open encoder with options
        let encoder = video_encoder.open_with(opts).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to open encoder: {}", e))
        })?;

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

        Ok(Self {
            output_path,
            encoder,
            octx,
            stream_index,
            frame_count: 0,
            width,
            height,
            fps,
            last_packet_keyframe: false,
            last_packet_pts: None,
            last_packet_dts: None,
        })
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
        // Just use frame count as PTS since encoder time_base is 1/fps
        yuv_frame.set_pts(Some(self.frame_count as i64));

        // Convert RGB to YUV420P
        Self::rgb_to_yuv420p(&processed_data, self.width, self.height, &mut yuv_frame)?;

        // Send frame to encoder
        self.encoder.send_frame(&yuv_frame).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to send frame: {}", e))
        })?;

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
        let q = quality.clamp(1, 10) as i32;
        let mapped = 42 - q * 3;
        mapped.clamp(12, 35) as u8
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
    mut shutdown_rx: Option<tokio::sync::oneshot::Receiver<()>>,
) -> Result<Vec<RecordingOutput>> {
    log::info!("Starting chunked frame processing with {}-second chunks", chunk_duration_secs);

    let mut chunk_outputs = Vec::new();
    let mut chunk_index = 0i64;
    let frames_per_chunk = (fps as u64) * chunk_duration_secs;
    let mut frames_in_current_chunk = 0u64;

    // Create first chunk
    let now = chrono::Local::now();
    let chunk_filename = format!("{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
    let chunk_path = base_output_dir.join(&chunk_filename);

    log::info!("Creating chunk {}: {}", chunk_index, chunk_path.display());

    let mut current_encoder = VideoEncoder::new(
        &chunk_path,
        width,
        height,
        fps,
        quality,
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

                            // Finish current encoder
                            let output = current_encoder.finish()?;
                            chunk_outputs.push(output);

                            // Start new chunk
                            chunk_index += 1;
                            frames_in_current_chunk = 0;

                            let now = chrono::Local::now();
                            let chunk_filename = format!("{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
                            let chunk_path = base_output_dir.join(&chunk_filename);

                            log::info!("Creating chunk {}: {}", chunk_index, chunk_path.display());

                            current_encoder = VideoEncoder::new(
                                &chunk_path,
                                width,
                                height,
                                fps,
                                quality,
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
                                ).await {
                                    log::error!("Failed to insert video chunk into database: {}", e);
                                }
                            }
                        }

                        // Encode frame and get metadata
                        let metadata = current_encoder.encode_frame(frame)?;
                        frames_in_current_chunk += 1;

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
                    log::info!("Finishing chunk {} with {} frames", chunk_index, frames_in_current_chunk);

                    // Finish current encoder
                    let output = current_encoder.finish()?;
                    chunk_outputs.push(output);

                    // Start new chunk
                    chunk_index += 1;
                    frames_in_current_chunk = 0;

                    let now = chrono::Local::now();
                    let chunk_filename = format!("{}.mp4", now.format("%Y-%m-%d_%H-%M-%S"));
                    let chunk_path = base_output_dir.join(&chunk_filename);

                    log::info!("Creating chunk {}: {}", chunk_index, chunk_path.display());

                    current_encoder = VideoEncoder::new(
                        &chunk_path,
                        width,
                        height,
                        fps,
                        quality,
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
                        ).await {
                            log::error!("Failed to insert video chunk into database: {}", e);
                        }
                    }
                }

                // Encode frame and get metadata
                let metadata = current_encoder.encode_frame(frame)?;
                frames_in_current_chunk += 1;

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

    log::info!("Chunked encoding complete: {} chunks created", chunk_outputs.len());
    Ok(chunk_outputs)
}

/// Process audio samples (currently just logs them, can be extended to save audio file)
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
