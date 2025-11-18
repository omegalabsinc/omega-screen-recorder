mod audio;
mod capture;
mod cli;
mod db;
mod display_info;
mod encoder;
mod error;
mod ffmpeg_utils;
mod interactions;
mod screenshot;

use crate::audio::AudioCapture;
use crate::capture::ScreenCapture;
use crate::cli::{Cli, Commands, RecordingType};
use crate::db::Database;
use crate::error::Result;
use crate::interactions::InteractionTracker;
use clap::Parser;
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let cli = Cli::parse();

    // Initialize logger
    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    log::info!("🎯 Omega Focus Screen Recorder v0.1.0");
    log::info!("================================================");

    // Execute the requested command
    match cli.command {
        Commands::Screenshot { output, display } => {
            log::info!("Taking screenshot...");
            screenshot::capture_screenshot(&output, display)?;
            println!("✅ Screenshot saved to: {}", output.display());
        }

        Commands::Concat { task_id, output, ffmpeg_path } => {
            log::info!("Concatenating chunks for task_id: {}", task_id);
            concatenate_chunks(&task_id, output, ffmpeg_path).await?;
        }

        Commands::Record {
            output,
            duration,
            fps,
            audio,
            no_audio,
            width,
            height,
            display,
            quality,
            track_interactions,
            track_mouse_moves,
            recording_type,
            task_id,
            chunk_duration,
            monitor_switch_interval,
            ffmpeg_path,
        } => {
            // Handle no_audio flag
            let audio = if no_audio {
                cli::AudioSource::None
            } else {
                audio
            };

            // Find and validate FFmpeg binary
            let ffmpeg_binary = ffmpeg_utils::find_ffmpeg_binary(ffmpeg_path.as_ref())?;

            // Validate FFmpeg is working
            match ffmpeg_utils::validate_ffmpeg(&ffmpeg_binary) {
                Ok(version) => {
                    log::info!("Using FFmpeg: {}", version);
                }
                Err(e) => {
                    return Err(e);
                }
            }
            // Validate recording type requirements
            if recording_type == RecordingType::Task {
                if task_id.is_none() {
                    return Err(error::ScreenRecError::InvalidParameter(
                        "task_id is required when recording_type is 'task'".to_string(),
                    ));
                }
            }

            // Validate FPS
            if fps == 0 || fps > 60 {
                return Err(error::ScreenRecError::InvalidParameter(
                    "FPS must be between 1 and 60".to_string(),
                ));
            }

            // Set up default output directory (~/.omega/data/)
            let omega_dir = dirs::home_dir()
                .ok_or_else(|| error::ScreenRecError::ConfigError("Could not find home directory".to_string()))?
                .join(".omega");

            let data_dir = omega_dir.join("data");
            let db_path = omega_dir.join("db.sqlite");

            // Determine output directory based on recording type
            let output_dir = if let Some(custom_output) = output {
                custom_output
            } else {
                match recording_type {
                    RecordingType::AlwaysOn => data_dir.join("always_on"),
                    RecordingType::Task => {
                        let tid = task_id.as_ref().unwrap();
                        data_dir.join("tasks").join(tid)
                    }
                }
            };

            // Create necessary directories
            std::fs::create_dir_all(&output_dir).map_err(|e| {
                error::ScreenRecError::ConfigError(format!("Failed to create output directory: {}", e))
            })?;

            log::info!("Starting screen recording...");
            log::info!("  Recording type: {}", recording_type);
            if let Some(tid) = &task_id {
                log::info!("  Task ID: {}", tid);
            }
            log::info!("  Output: {}", output_dir.display());
            log::info!("  Chunk duration: {} seconds", chunk_duration);
            log::info!("  FPS: {}", fps);
            log::info!(
                "  Duration: {}",
                if duration > 0 {
                    format!("{} seconds", duration)
                } else {
                    "unlimited (Ctrl+C to stop)".to_string()
                }
            );
            log::info!("  Audio: {}", audio);
            log::info!("  Quality: {}/10", quality);
            log::info!(
                "  Interaction tracking: {}",
                if track_interactions {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            log::info!("Initializing database at: {}", db_path.display());
            let db = Arc::new(Database::new(&db_path).await?);

            // Get device name (hostname)
            let device_name = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string());

            // Initialize screen capture
            let monitor_switch_duration = std::time::Duration::from_secs_f64(monitor_switch_interval);
            let screen_capture = ScreenCapture::new(display, fps, monitor_switch_duration)?;

            let mut capture_width = if width > 0 {
                width as usize
            } else if screen_capture.is_multi_monitor() {
                // In multi-monitor mode, use maximum dimensions across all displays
                let (max_w, _) = screen_capture.get_max_dimensions()?;
                log::info!("Multi-monitor mode: using maximum width {}", max_w);
                max_w
            } else {
                screen_capture.width()
            };

            let mut capture_height = if height > 0 {
                height as usize
            } else if screen_capture.is_multi_monitor() {
                // In multi-monitor mode, use maximum dimensions across all displays
                let (_, max_h) = screen_capture.get_max_dimensions()?;
                log::info!("Multi-monitor mode: using maximum height {}", max_h);
                max_h
            } else {
                screen_capture.height()
            };

            // Ensure dimensions are even (required by H.264 encoder)
            if capture_width % 2 != 0 {
                capture_width -= 1;
                log::info!("Adjusted width to even number: {}", capture_width);
            }
            if capture_height % 2 != 0 {
                capture_height -= 1;
                log::info!("Adjusted height to even number: {}", capture_height);
            }

            log::info!("Capture resolution: {}x{}", capture_width, capture_height);

            // Create channels for frame and audio data
            let (frame_tx_std, frame_rx_std) = std_mpsc::channel(); // Sync channel for capture thread
            let (frame_tx, frame_rx) = mpsc::channel(60); // Async channel for encoder
            let (audio_tx, audio_rx) = mpsc::channel(1000);

            // Bridge: sync receiver -> async sender
            let bridge_handle = tokio::spawn(async move {
                while let Ok(frame) = frame_rx_std.recv() {
                    if frame_tx.send(frame).await.is_err() {
                        break;
                    }
                }
            });

            // Start encoder task with chunking support
            let db_for_encoder = db.clone();
            let device_name_for_encoder = device_name.clone();
            let output_dir_for_encoder = output_dir.clone();
            let recording_type_str = recording_type.to_string();
            let task_id_for_encoder = task_id.clone();

            let encoder_handle = tokio::spawn(async move {
                encoder::process_frames_chunked(
                    frame_rx,
                    output_dir_for_encoder,
                    capture_width,
                    capture_height,
                    fps,
                    quality,
                    chunk_duration,
                    Some(db_for_encoder),
                    Some(device_name_for_encoder),
                    Some(recording_type_str),
                    task_id_for_encoder,
                )
                .await
            });

            // Initialize audio capture if requested
            let audio_handle = if audio != cli::AudioSource::None {
                match AudioCapture::new(audio)? {
                    Some(audio_capture) => {
                        // Start audio capture in a separate thread (cpal requires non-async)
                        let audio_tx_clone = audio_tx.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = audio_capture.start_capture(audio_tx_clone) {
                                log::error!("Audio capture failed: {}", e);
                            }
                        });

                        // Start audio processing task
                        Some(tokio::spawn(async move {
                            encoder::process_audio(audio_rx).await
                        }))
                    }
                    None => {
                        log::info!("Audio capture disabled");
                        None
                    }
                }
            } else {
                None
            };

            // Initialize interaction tracker
            // For task mode: always track clicks to JSONL
            // For always_on mode: only track if --track-interactions is enabled
            // Note: The interaction tracker also handles cursor position updates
            let interaction_tracker = if recording_type == RecordingType::Task && task_id.is_some() {
                // Task mode: always track clicks to JSONL
                let tid = task_id.as_ref().unwrap();
                let jsonl_path = output_dir.join("clicks.jsonl");
                log::info!("Task mode: Click tracking enabled -> {}", jsonl_path.display());

                let tracker = InteractionTracker::new_for_task(
                    capture_width,
                    capture_height,
                    track_mouse_moves,
                    tid.clone(),
                    jsonl_path,
                )?;

                let tracker_handle = tracker.start()?;
                Some((tracker, tracker_handle))
            } else if track_interactions {
                // Always_on mode: only track if explicitly requested
                let tracker = InteractionTracker::new(capture_width, capture_height, track_mouse_moves);
                let tracker_handle = tracker.start()?;
                Some((tracker, tracker_handle))
            } else {
                // No interaction tracking, but still need cursor updates for rendering
                let _cursor_tracker_handle = std::thread::spawn(|| {
                    use crate::capture::update_cursor_position;
                    let _ = rdev::listen(move |event| {
                        if let rdev::EventType::MouseMove { x, y } = event.event_type {
                            update_cursor_position(x as i32, y as i32);
                        }
                    });
                });
                None
            };

            // Set up Ctrl+C handler for graceful shutdown
            let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
            let r = running.clone();
            ctrlc::set_handler(move || {
                log::info!("Received Ctrl+C, stopping recording...");
                r.store(false, std::sync::atomic::Ordering::SeqCst);
            })
            .map_err(|e| {
                error::ScreenRecError::ConfigError(format!("Failed to set Ctrl+C handler: {}", e))
            })?;

            // Calculate target frames based on duration and fps
            let target_frames = if duration > 0 {
                Some(duration * fps as u64)
            } else {
                None
            };

            // Run capture in a separate OS thread (not tokio thread) because Capturer is not Send
            let running_for_capture = running.clone();
            let capture_handle = std::thread::spawn(move || {
                screen_capture.start_capture_sync(frame_tx_std, target_frames, Some(running_for_capture))
            });

            // Wait for capture to finish
            let _capture_result = capture_handle
                .join()
                .map_err(|e| {
                    error::ScreenRecError::CaptureError(format!("Capture thread panicked: {:?}", e))
                })?
                .map_err(|e| {
                    error::ScreenRecError::CaptureError(format!("Capture failed: {}", e))
                })?;

            // Wait for bridge to finish
            let _ = bridge_handle.await;

            // Wait for encoder to finish and get video chunks
            let chunk_outputs = encoder_handle.await.map_err(|e| {
                error::ScreenRecError::EncodingError(format!("Encoder task failed: {}", e))
            })??;

            // Wait for audio processing if it was started
            if let Some(handle) = audio_handle {
                let _ = handle.await;
            }

            // Save interaction data if tracking was enabled
            if let Some((tracker, _handle)) = interaction_tracker {
                let interactions_path = output_dir.join("interactions.json");

                log::info!("Saving interaction data...");
                if let Err(e) = tracker.save(&interactions_path) {
                    log::error!("Failed to save interaction data: {}", e);
                } else {
                    println!("✅ Interactions saved to: {}", interactions_path.display());
                }
            }

            // Log where chunks were saved
            log::info!("Recording completed. Chunks saved to: {}", output_dir.display());
            println!("✅ Recording saved to: {}", output_dir.display());
            println!("   {} chunk(s) created", chunk_outputs.len());

            // If this is a task recording, inform about concatenation
            if recording_type == RecordingType::Task {
                if let Some(tid) = task_id {
                    println!("\n💡 To concatenate chunks into a final video, run:");
                    println!("   omgrec concat --task-id {}", tid);
                }
            }

            log::info!("Recording completed successfully");
        }
    }

    Ok(())
}

/// Concatenate video chunks for a completed task recording
async fn concatenate_chunks(
    task_id: &str,
    output_path: Option<std::path::PathBuf>,
    ffmpeg_path: Option<std::path::PathBuf>,
) -> Result<()> {
    println!("🔄 [PROGRESS] Starting concatenation for task: {}", task_id);
    log::info!("Starting chunk concatenation for task_id: {}", task_id);

    // Find and validate FFmpeg binary
    println!("🔄 [PROGRESS] Validating FFmpeg installation...");
    let ffmpeg_binary = ffmpeg_utils::find_ffmpeg_binary(ffmpeg_path.as_ref())?;

    // Validate FFmpeg is working
    match ffmpeg_utils::validate_ffmpeg(&ffmpeg_binary) {
        Ok(version) => {
            println!("✅ [PROGRESS] FFmpeg validated: {}", version);
            log::info!("Using FFmpeg: {}", version);
        }
        Err(e) => {
            return Err(e);
        }
    }

    // Set up default output directory (~/.omega/data/)
    let omega_dir = dirs::home_dir()
        .ok_or_else(|| error::ScreenRecError::ConfigError("Could not find home directory".to_string()))?
        .join(".omega");

    let data_dir = omega_dir.join("data");

    // Initialize database
    println!("🔄 [PROGRESS] Loading recording data from database...");
    let db_path = omega_dir.join("db.sqlite");
    let db = Database::new(&db_path).await?;

    // Get all chunks for this task from database
    let chunks = db.get_chunks_by_task_id(task_id).await?;

    if chunks.is_empty() {
        log::warn!("No chunks found for task_id: {}", task_id);
        return Err(error::ScreenRecError::InvalidParameter(
            format!("No chunks found for task_id: {}", task_id)
        ));
    }

    println!("✅ [PROGRESS] Found {} video chunks to concatenate", chunks.len());
    log::info!("Found {} chunks to concatenate", chunks.len());

    // Determine output directory from first chunk
    let first_chunk_path = std::path::Path::new(&chunks[0].file_path);
    let output_dir = if first_chunk_path.is_absolute() {
        first_chunk_path.parent()
            .ok_or_else(|| error::ScreenRecError::ConfigError("Could not determine output directory".to_string()))?
            .to_path_buf()
    } else {
        data_dir.join(first_chunk_path.parent()
            .ok_or_else(|| error::ScreenRecError::ConfigError("Could not determine output directory".to_string()))?)
    };

    log::info!("Output directory: {}", output_dir.display());

    // Check if we need normalization (multiple resolutions detected)
    println!("🔄 [PROGRESS] Analyzing video frames and resolutions...");
    let frames = db.get_frames_by_task_id(task_id).await?;
    let mut resolutions = std::collections::HashSet::new();
    for frame in &frames {
        if let (Some(w), Some(h)) = (frame.display_width, frame.display_height) {
            resolutions.insert((w, h));
        }
    }

    let needs_normalization = resolutions.len() > 1;

    if needs_normalization {
        println!("⚠️  [PROGRESS] Multiple resolutions detected - normalization required");
        log::info!("Multiple resolutions detected: {:?}", resolutions);
        log::info!("Video normalization will be applied during concatenation");

        // Find the maximum dimensions across all resolutions
        let (max_width, max_height) = resolutions.iter()
            .fold((0i64, 0i64), |(max_w, max_h), &(w, h)| {
                (max_w.max(w), max_h.max(h))
            });

        println!("📐 [PROGRESS] Target resolution: {}x{}", max_width, max_height);
        log::info!("Target resolution: {}x{}", max_width, max_height);
    } else {
        println!("✅ [PROGRESS] Single resolution detected - fast concatenation mode");
        log::info!("Single resolution detected, no normalization needed");
    }

    // Create concat file list for FFmpeg
    println!("🔄 [PROGRESS] Preparing concatenation list...");
    let concat_list_path = output_dir.join("concat_list.txt");
    let mut concat_content = String::new();
    let mut existing_chunks = 0;
    let mut missing_chunks = 0;

    for chunk in &chunks {
        // Build absolute path to chunk file
        let chunk_path = if std::path::Path::new(&chunk.file_path).is_absolute() {
            std::path::PathBuf::from(&chunk.file_path)
        } else {
            data_dir.join(&chunk.file_path)
        };

        // Only include files that actually exist
        if chunk_path.exists() {
            // Escape single quotes in the path by replacing ' with '\''
            let path_str = chunk_path.to_string_lossy().replace("'", r"'\''");
            concat_content.push_str(&format!("file '{}'\n", path_str));
            existing_chunks += 1;
        } else {
            log::warn!("Skipping missing chunk file: {}", chunk_path.display());
            missing_chunks += 1;
        }
    }

    if existing_chunks == 0 {
        return Err(error::ScreenRecError::InvalidParameter(
            format!("No valid chunk files found for task_id: {}", task_id)
        ));
    }

    if missing_chunks > 0 {
        println!("⚠️  [PROGRESS] Warning: {} chunk files missing, using {} available chunks",
                 missing_chunks, existing_chunks);
        log::warn!("{} chunk files are missing, concatenating {} existing chunks",
                   missing_chunks, existing_chunks);
    }

    std::fs::write(&concat_list_path, concat_content).map_err(|e| {
        error::ScreenRecError::EncodingError(format!("Failed to write concat list: {}", e))
    })?;

    // Determine final output path
    let final_output_path = output_path.unwrap_or_else(|| output_dir.join("final.mp4"));
    println!("🎬 [PROGRESS] Starting FFmpeg concatenation...");
    println!("   Output: {}", final_output_path.display());
    log::info!("Concatenating chunks to: {}", final_output_path.display());

    let mut ffmpeg_args = vec![
        "-f".to_string(), "concat".to_string(),
        "-safe".to_string(), "0".to_string(),
        "-i".to_string(), concat_list_path.to_str().unwrap().to_string(),
    ];

    if needs_normalization {
        // Find the maximum dimensions
        let (max_width, max_height) = resolutions.iter()
            .fold((0i64, 0i64), |(max_w, max_h), &(w, h)| {
                (max_w.max(w), max_h.max(h))
            });

        // Add video filter for scaling and padding
        let filter_string = format!(
            "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:black",
            max_width, max_height, max_width, max_height
        );

        ffmpeg_args.extend(vec![
            "-vf".to_string(), filter_string,
            "-c:v".to_string(), "libx264".to_string(),
            "-preset".to_string(), "medium".to_string(),
            "-crf".to_string(), "23".to_string(),
        ]);
    } else {
        // No normalization needed, use copy
        ffmpeg_args.extend(vec![
            "-c".to_string(), "copy".to_string(),
        ]);
    }

    ffmpeg_args.push(final_output_path.to_str().unwrap().to_string());

    log::info!("Running FFmpeg concatenation: {}", ffmpeg_binary);

    let concat_result = std::process::Command::new(&ffmpeg_binary)
        .args(&ffmpeg_args)
        .output()
        .map_err(|e| {
            error::ScreenRecError::EncodingError(format!("Failed to run ffmpeg concat: {}", e))
        })?;

    if !concat_result.status.success() {
        let stderr = String::from_utf8_lossy(&concat_result.stderr);
        println!("❌ [PROGRESS] FFmpeg concatenation failed");
        return Err(error::ScreenRecError::EncodingError(format!(
            "FFmpeg concat failed: {}",
            stderr
        )));
    }

    // Clean up concat list file
    let _ = std::fs::remove_file(&concat_list_path);

    println!("✅ [PROGRESS] Video concatenation complete!");
    log::info!("✅ Final video created: {}", final_output_path.display());
    println!("✅ Final video saved to: {}", final_output_path.display());

    // Get video metadata using ffprobe
    println!("🔄 [PROGRESS] Extracting video metadata...");
    log::info!("Extracting video metadata...");
    let ffprobe_cmd = ffmpeg_utils::find_ffprobe_binary(&ffmpeg_binary);

    let ffprobe_result = std::process::Command::new(&ffprobe_cmd)
        .args(&[
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
            final_output_path.to_str().unwrap()
        ])
        .output();

    let mut video_duration_secs = 0.0;
    let mut video_bitrate = 0;
    let mut video_codec = String::new();
    let mut file_size_bytes = 0;

    if let Ok(result) = ffprobe_result {
        if result.status.success() {
            let output_str = String::from_utf8_lossy(&result.stdout);
            if let Ok(json_data) = serde_json::from_str::<serde_json::Value>(&output_str) {
                // Get duration from format
                if let Some(format_obj) = json_data.get("format") {
                    if let Some(duration_str) = format_obj.get("duration").and_then(|v| v.as_str()) {
                        video_duration_secs = duration_str.parse::<f64>().unwrap_or(0.0);
                    }
                    if let Some(bitrate_str) = format_obj.get("bit_rate").and_then(|v| v.as_str()) {
                        video_bitrate = bitrate_str.parse::<i64>().unwrap_or(0);
                    }
                    if let Some(size_str) = format_obj.get("size").and_then(|v| v.as_str()) {
                        file_size_bytes = size_str.parse::<i64>().unwrap_or(0);
                    }
                }
                // Get codec from first video stream
                if let Some(streams) = json_data.get("streams").and_then(|v| v.as_array()) {
                    for stream in streams {
                        if stream.get("codec_type").and_then(|v| v.as_str()) == Some("video") {
                            video_codec = stream.get("codec_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            break;
                        }
                    }
                }
            }
        }
    }

    // Get device name and recording settings from first chunk
    let device_name = "unknown".to_string(); // We don't have this info readily available
    let fps = 30; // Default, we don't have this stored per task
    let quality = 8; // Default

    println!("🔄 [PROGRESS] Calculating recording statistics...");

    // Calculate monitor statistics
    let unique_displays: std::collections::HashSet<_> = frames.iter()
        .filter_map(|f| f.display_index)
        .collect();
    let num_monitors_used = unique_displays.len();

    // Calculate resolution statistics
    let mut resolution_usage: std::collections::HashMap<(i64, i64), usize> = std::collections::HashMap::new();
    for frame in &frames {
        if let (Some(w), Some(h)) = (frame.display_width, frame.display_height) {
            *resolution_usage.entry((w, h)).or_insert(0) += 1;
        }
    }

    // Calculate keyframe statistics
    let keyframe_count = frames.iter().filter(|f| f.is_keyframe == 1).count();

    // Get chunk details
    let chunk_details: Vec<serde_json::Value> = chunks.iter().map(|c| {
        serde_json::json!({
            "chunk_index": c.chunk_index,
            "file_path": c.file_path,
            "created_at": c.created_at.to_rfc3339(),
        })
    }).collect();

    // Export comprehensive metadata to JSON
    println!("🔄 [PROGRESS] Generating metadata files...");
    log::info!("Exporting comprehensive metadata to JSON...");

    let metadata_output = serde_json::json!({
        "version": "1.0",
        "task_id": task_id,
        "device_name": device_name,
        "recording_type": "task",
        "created_at": chunks.first().map(|c| c.created_at.to_rfc3339()).unwrap_or_default(),
        "video": {
            "final_video_path": final_output_path.file_name().and_then(|n| n.to_str()).unwrap_or("final.mp4"),
            "duration_seconds": video_duration_secs,
            "duration_formatted": format!("{}h {}m {:.1}s",
                (video_duration_secs / 3600.0) as i64,
                ((video_duration_secs % 3600.0) / 60.0) as i64,
                video_duration_secs % 60.0
            ),
            "file_size_bytes": file_size_bytes,
            "file_size_mb": format!("{:.2}", file_size_bytes as f64 / 1024.0 / 1024.0),
            "codec": video_codec,
            "bitrate_bps": video_bitrate,
            "fps": fps,
            "quality": quality,
        },
        "focused_time": {
            "total_seconds": video_duration_secs,
            "formatted": format!("{}h {}m {:.1}s",
                (video_duration_secs / 3600.0) as i64,
                ((video_duration_secs % 3600.0) / 60.0) as i64,
                video_duration_secs % 60.0
            ),
            "total_minutes": format!("{:.2}", video_duration_secs / 60.0),
            "total_hours": format!("{:.3}", video_duration_secs / 3600.0),
        },
        "chunks": {
            "total_count": chunks.len(),
            "details": chunk_details,
        },
        "frames": {
            "total_count": frames.len(),
            "keyframe_count": keyframe_count,
            "keyframe_interval": if keyframe_count > 0 { frames.len() / keyframe_count } else { 0 },
        },
        "displays": {
            "monitors_used": num_monitors_used,
            "unique_display_indices": unique_displays.iter().cloned().collect::<Vec<_>>(),
            "normalized": needs_normalization,
            "resolutions": resolutions.iter().map(|(w, h)| {
                serde_json::json!({
                    "width": w,
                    "height": h,
                    "frame_count": resolution_usage.get(&(*w, *h)).unwrap_or(&0),
                })
            }).collect::<Vec<_>>(),
            "final_resolution": if needs_normalization {
                let (max_width, max_height) = resolutions.iter()
                    .fold((0i64, 0i64), |(max_w, max_h), &(w, h)| {
                        (max_w.max(w), max_h.max(h))
                    });
                serde_json::json!({
                    "width": max_width,
                    "height": max_height,
                })
            } else {
                resolutions.iter().next()
                    .map(|(w, h)| serde_json::json!({
                        "width": w,
                        "height": h,
                    }))
                    .unwrap_or(serde_json::json!({}))
            },
        },
    });

    let metadata_path = output_dir.join("metadata.json");
    std::fs::write(&metadata_path, serde_json::to_string_pretty(&metadata_output).unwrap())
        .map_err(|e| {
            error::ScreenRecError::EncodingError(format!("Failed to write metadata JSON: {}", e))
        })?;

    println!("✅ [PROGRESS] Metadata file created");
    log::info!("✅ Metadata exported: {}", metadata_path.display());
    println!("   📄 {}", metadata_path.display());

    // Also export detailed frame metadata to a separate JSON
    println!("🔄 [PROGRESS] Exporting frame-level data...");
    log::info!("Exporting detailed frame metadata to JSON...");

    let frames_output = serde_json::json!({
        "task_id": task_id,
        "total_frames": frames.len(),
        "frames": frames.iter().map(|f| {
            serde_json::json!({
                "offset": f.offset_index,
                "timestamp": f.timestamp.to_rfc3339(),
                "pts": f.pts,
                "is_keyframe": f.is_keyframe == 1,
                "display_index": f.display_index,
                "display_width": f.display_width,
                "display_height": f.display_height,
            })
        }).collect::<Vec<_>>()
    });

    let frames_path = output_dir.join(format!("{}_frames.json", task_id));
    std::fs::write(&frames_path, serde_json::to_string_pretty(&frames_output).unwrap())
        .map_err(|e| {
            error::ScreenRecError::EncodingError(format!("Failed to write frames JSON: {}", e))
        })?;

    println!("✅ [PROGRESS] Frame data exported ({} frames)", frames.len());
    log::info!("✅ Detailed frame metadata exported: {}", frames_path.display());
    println!("   📄 {}", frames_path.display());

    println!("\n🎉 [PROGRESS] Concatenation complete!");
    println!("   Duration: {:.1}s | Size: {:.2}MB | Frames: {}",
        video_duration_secs,
        file_size_bytes as f64 / 1024.0 / 1024.0,
        frames.len()
    );

    Ok(())
}
