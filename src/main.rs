#[cfg(target_os = "macos")]
mod audio;
mod capture;
mod cli;
mod db;
mod display_info;
mod encoder;
#[cfg(target_os = "macos")]
mod encoder_subprocess;
mod error;
mod ffmpeg_utils;
mod interactions;
mod screenshot;

#[cfg(target_os = "macos")]
use crate::audio::AudioCapture;
use crate::capture::ScreenCapture;
use crate::cli::{Cli, Commands, RecordingType};
use crate::db::Database;
use crate::error::{Result, ScreenRecError};
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

        Commands::InspectSessions { task_id } => {
            log::info!("Inspecting sessions for task_id: {}", task_id);
            inspect_sessions(&task_id).await?;
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

            log::info!("Initializing database at: {}", db_path.display());
            let db = Arc::new(Database::new(&db_path).await?);

            // Get device name (hostname)
            let device_name = hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "unknown".to_string());

            // Create recording session (capture start time)
            let session_start_time = chrono::Utc::now();
            let session_id = if recording_type == RecordingType::Task && task_id.is_some() {
                let tid = task_id.as_ref().unwrap();
                let id = db.create_recording_session(tid, &device_name, session_start_time).await?;
                log::info!("Created recording session {} for task {}", id, tid);
                Some(id)
            } else {
                None
            };

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

            // Create channels for frame data
            let (frame_tx_std, frame_rx_std) = std_mpsc::channel(); // Sync channel for capture thread
            // Increased buffer from 60 to 300 frames (10 seconds at 30fps) to prevent blocking during database writes
            let (frame_tx, frame_rx) = mpsc::channel(300); // Async channel for encoder

            // Bridge: sync receiver -> async sender (NO DROPS - blocks if encoder is slow)
            let bridge_handle = tokio::spawn(async move {
                let mut total_frames = 0u64;
                let mut last_log = std::time::Instant::now();

                while let Ok(frame) = frame_rx_std.recv() {
                    total_frames += 1;

                    // Log progress every 5 seconds
                    if last_log.elapsed() >= std::time::Duration::from_secs(5) {
                        log::info!("Bridge: {} frames forwarded to encoder", total_frames);
                        last_log = std::time::Instant::now();
                    }

                    // Send frame - will block if channel is full (encoder is slow)
                    // This is CORRECT - we want to preserve all frames, not drop them!
                    if frame_tx.send(frame).await.is_err() {
                        log::error!("Encoder channel closed unexpectedly");
                        break;
                    }
                }

                log::info!("Bridge completed: {} total frames forwarded", total_frames);
            });

            // Create shutdown channel for graceful encoder termination
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

            // Start encoder task with chunking support
            let db_for_encoder = db.clone();
            let device_name_for_encoder = device_name.clone();
            let output_dir_for_encoder = output_dir.clone();
            let recording_type_str = recording_type.to_string();
            let task_id_for_encoder = task_id.clone();

            #[cfg(target_os = "macos")]
            let encoder_handle = {
                let ffmpeg_path_for_encoder = Some(ffmpeg_binary.clone());
                tokio::spawn(async move {
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
                        session_id,
                        Some(shutdown_rx),
                        ffmpeg_path_for_encoder,
                    )
                    .await
                })
            };

            #[cfg(not(target_os = "macos"))]
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
                    session_id,
                    Some(shutdown_rx),
                )
                .await
            });

            // Initialize audio capture if requested (macOS only)
            #[cfg(target_os = "macos")]
            let audio_handle = if audio != cli::AudioSource::None {
                match AudioCapture::new(audio) {
                    Ok(Some(audio_capture)) => {
                        let (audio_tx, audio_rx) = mpsc::channel(1000);
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
                    Ok(None) => {
                        log::info!("Audio capture disabled");
                        None
                    }
                    Err(e) => {
                        // Audio failed, but continue with video-only recording
                        match e {
                            ScreenRecError::AudioDeviceUnavailable(tried) => {
                                log::warn!("Audio unavailable (tried: {:?}). Continuing with video only.", tried);
                                println!("⚠️  Audio capture failed. Recording video only.");
                            }
                            _ => {
                                log::warn!("Audio init failed: {}. Continuing with video only.", e);
                                println!("⚠️  Audio initialization failed. Recording video only.");
                            }
                        }
                        None
                    }
                }
            } else {
                None
            };

            // Audio not supported on Windows yet
            #[cfg(not(target_os = "macos"))]
            let audio_handle: Option<tokio::task::JoinHandle<()>> = {
                if audio != cli::AudioSource::None {
                    log::warn!("Audio capture is only supported on macOS");
                }
                None
            };

            // Initialize interaction tracker
            // For task mode: always track all interactions (clicks, keys, scrolls) to JSONL
            // For always_on mode: only track if --track-interactions is enabled
            // Note: The interaction tracker also handles cursor position updates
            let interaction_tracker = if recording_type == RecordingType::Task && task_id.is_some() {
                // Task mode: always track all interactions to JSONL
                let tid = task_id.as_ref().unwrap();
                let jsonl_path = output_dir.join("interactions.jsonl");
                log::info!("Task mode: Interaction tracking enabled -> {}", jsonl_path.display());

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

            // Wrap shutdown_tx in Arc<Mutex<Option<_>>> so we can move it into the handler
            let shutdown_tx_for_handler = Arc::new(std::sync::Mutex::new(Some(shutdown_tx)));
            let shutdown_tx_clone = shutdown_tx_for_handler.clone();

            // Clone db and session_id for signal handler
            let db_for_ctrlc = db.clone();
            let session_id_for_ctrlc = session_id;

            ctrlc::set_handler(move || {
                log::info!("Received Ctrl+C, stopping recording...");

                // End recording session immediately
                if let Some(sid) = session_id_for_ctrlc {
                    let session_end_time = chrono::Utc::now();
                    let db_clone = db_for_ctrlc.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            if let Err(e) = db_clone.end_recording_session(sid, session_end_time).await {
                                log::error!("Failed to update recording session end time in Ctrl+C handler: {}", e);
                            } else {
                                log::info!("Recording session {} ended via Ctrl+C", sid);
                            }
                        });
                    });
                }

                // Signal encoder to finish current chunk
                if let Ok(mut tx_opt) = shutdown_tx_clone.lock() {
                    if let Some(tx) = tx_opt.take() {
                        log::info!("Signaling encoder to finalize current chunk...");
                        let _ = tx.send(());
                        // Give encoder a moment to finish
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                }

                // Then signal capture to stop
                r.store(false, std::sync::atomic::Ordering::SeqCst);
            })
            .map_err(|e| {
                error::ScreenRecError::ConfigError(format!("Failed to set Ctrl+C handler: {}", e))
            })?;

            // Also handle SIGTERM (Unix only) for graceful shutdown on kill
            #[cfg(unix)]
            {
                let running_sigterm = running.clone();
                let shutdown_tx_sigterm = shutdown_tx_for_handler.clone();
                let db_for_sigterm = db.clone();
                let session_id_for_sigterm = session_id;

                tokio::spawn(async move {
                    use tokio::signal::unix::{signal, SignalKind};
                    let mut sigterm = signal(SignalKind::terminate())
                        .expect("Failed to register SIGTERM handler");

                    sigterm.recv().await;
                    log::info!("Received SIGTERM, stopping recording...");

                    // End recording session immediately
                    if let Some(sid) = session_id_for_sigterm {
                        let session_end_time = chrono::Utc::now();
                        if let Err(e) = db_for_sigterm.end_recording_session(sid, session_end_time).await {
                            log::error!("Failed to update recording session end time in SIGTERM handler: {}", e);
                        } else {
                            log::info!("Recording session {} ended via SIGTERM", sid);
                        }
                    }

                    // Signal encoder to finish current chunk
                    {
                        if let Ok(mut tx_opt) = shutdown_tx_sigterm.lock() {
                            if let Some(tx) = tx_opt.take() {
                                log::info!("Signaling encoder to finalize current chunk...");
                                let _ = tx.send(());
                            }
                        }
                        // Guard is dropped here before the await
                    }

                    // Give encoder a moment to finish
                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                    // Then signal capture to stop
                    running_sigterm.store(false, std::sync::atomic::Ordering::SeqCst);
                });
            }

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

            // Update recording session end time
            if let Some(sid) = session_id {
                let session_end_time = chrono::Utc::now();
                if let Err(e) = db.end_recording_session(sid, session_end_time).await {
                    log::error!("Failed to update recording session end time: {}", e);
                } else {
                    let duration = (session_end_time - session_start_time).num_seconds();
                    log::info!("Recording session {} ended. Duration: {}s", sid, duration);
                }
            }

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
                    println!("   screenrec concat --task-id {}", tid);
                }
            }

            log::info!("Recording completed successfully");
        }
    }

    Ok(())
}

/// Concatenate video chunks for a completed task recording
async fn inspect_sessions(task_id: &str) -> Result<()> {
    println!("🔍 Inspecting recording sessions for task: {}", task_id);

    // Set up database path
    let omega_dir = dirs::home_dir()
        .ok_or_else(|| error::ScreenRecError::ConfigError("Could not find home directory".to_string()))?
        .join(".omega");

    let db_path = omega_dir.join("db.sqlite");
    let db = Database::new(&db_path).await?;

    // Get all sessions for this task
    let sessions = db.get_sessions_for_task(task_id).await?;

    if sessions.is_empty() {
        println!("❌ No recording sessions found for task_id: {}", task_id);
        return Ok(());
    }

    println!("\n📊 Found {} recording session(s):\n", sessions.len());
    println!("{:<6} {:<22} {:<22} {:<12} {:<15}",
             "ID", "Started At", "Ended At", "Duration (s)", "Status");
    println!("{}", "=".repeat(80));

    let mut total_duration = 0.0;

    for session in &sessions {
        let duration = if let Some(ended_at) = session.ended_at {
            let duration_secs = (ended_at - session.started_at).num_seconds() as f64;
            total_duration += duration_secs;
            format!("{:.2}", duration_secs)
        } else {
            "N/A".to_string()
        };

        let status = if session.ended_at.is_some() {
            "Completed"
        } else {
            "In Progress"
        };

        let ended_at_str = if let Some(ended_at) = session.ended_at {
            ended_at.format("%Y-%m-%d %H:%M:%S").to_string()
        } else {
            "N/A".to_string()
        };

        println!("{:<6} {:<22} {:<22} {:<12} {:<15}",
                 session.id,
                 session.started_at.format("%Y-%m-%d %H:%M:%S"),
                 ended_at_str,
                 duration,
                 status);
    }

    println!("{}", "=".repeat(80));
    println!("\n✅ Total accumulated recording time: {:.2} seconds ({:.2} minutes)",
             total_duration, total_duration / 60.0);

    // Also get and display the database-calculated total for verification
    let db_total = db.get_total_recording_time(task_id).await?;
    println!("✅ Database calculated total: {:.2} seconds ({:.2} minutes)",
             db_total, db_total / 60.0);

    if (total_duration - db_total).abs() > 0.01 {
        println!("⚠️  WARNING: Mismatch between manual sum and database calculation!");
    }

    Ok(())
}

async fn concatenate_chunks(
    task_id: &str,
    output_path: Option<std::path::PathBuf>,
    ffmpeg_path: Option<std::path::PathBuf>,
) -> Result<()> {
    const MAX_RETRIES: u32 = 3;
    let mut last_error = None;

    for attempt in 1..=MAX_RETRIES {
        if attempt > 1 {
            log::info!("Retry attempt {}/{} for task {}", attempt, MAX_RETRIES, task_id);
            println!("🔄 [PROGRESS] Retry attempt {}/{}", attempt, MAX_RETRIES);

            // Wait before retrying (exponential backoff: 2s, 4s, 8s)
            let wait_secs = 2u64.pow(attempt - 1);
            log::info!("Waiting {}s before retry...", wait_secs);
            tokio::time::sleep(tokio::time::Duration::from_secs(wait_secs)).await;
        }

        println!("🔄 [PROGRESS] Starting concatenation for task: {} (attempt {}/{})", task_id, attempt, MAX_RETRIES);
        log::info!("Starting chunk concatenation for task_id: {} (attempt {}/{})", task_id, attempt, MAX_RETRIES);

        match concatenate_chunks_impl(task_id, output_path.clone(), ffmpeg_path.clone()).await {
            Ok(()) => {
                if attempt > 1 {
                    log::info!("✅ Concatenation succeeded on attempt {}/{}", attempt, MAX_RETRIES);
                }
                return Ok(());
            }
            Err(e) => {
                log::error!("Concatenation attempt {}/{} failed: {}", attempt, MAX_RETRIES, e);
                println!("❌ [PROGRESS] Attempt {}/{} failed: {}", attempt, MAX_RETRIES, e);
                last_error = Some(e);

                if attempt < MAX_RETRIES {
                    log::warn!("Will retry concatenation...");
                }
            }
        }
    }

    // All retries exhausted
    Err(last_error.unwrap_or_else(|| {
        error::ScreenRecError::EncodingError("All concatenation attempts failed".to_string())
    }))
}

async fn concatenate_chunks_impl(
    task_id: &str,
    output_path: Option<std::path::PathBuf>,
    ffmpeg_path: Option<std::path::PathBuf>,
) -> Result<()> {

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

    // Extract FPS from chunks (use first chunk's FPS, default to 30 if not set)
    let fps = chunks.iter()
        .find_map(|chunk| chunk.fps)
        .unwrap_or(30);
    log::info!("Using FPS: {}", fps);

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
    let mut invalid_chunks = 0;
    let mut total_chunk_duration = 0.0;

    // Get ffprobe path for validating chunks
    let ffprobe_cmd = ffmpeg_utils::find_ffprobe_binary(&ffmpeg_binary);

    log::info!("===== CHUNK VALIDATION =====");

    for (idx, chunk) in chunks.iter().enumerate() {
        // Build absolute path to chunk file
        let chunk_path = if std::path::Path::new(&chunk.file_path).is_absolute() {
            std::path::PathBuf::from(&chunk.file_path)
        } else {
            data_dir.join(&chunk.file_path)
        };

        // Only include files that actually exist
        if chunk_path.exists() {
            // Get file size first - skip extremely small files (likely corrupted)
            let file_size = std::fs::metadata(&chunk_path)
                .map(|m| m.len())
                .unwrap_or(0);

            if file_size < 1024 {
                log::warn!("Skipping chunk {} - file too small ({} bytes, likely corrupted): {}",
                    idx + 1, file_size, chunk_path.display());
                invalid_chunks += 1;
                continue;
            }

            // Get chunk duration
            let duration_result = std::process::Command::new(&ffprobe_cmd)
                .args(&[
                    "-v", "error",
                    "-show_entries", "format=duration",
                    "-of", "default=noprint_wrappers=1:nokey=1",
                    chunk_path.to_str().unwrap()
                ])
                .output();

            // Parse duration and validate it's a valid number
            let duration_opt = duration_result
                .ok()
                .and_then(|result| String::from_utf8(result.stdout).ok())
                .and_then(|duration_str| {
                    let trimmed = duration_str.trim();
                    // Check for "N/A" or invalid duration strings
                    if trimmed == "N/A" || trimmed.is_empty() {
                        None
                    } else {
                        trimmed.parse::<f64>().ok().and_then(|d| {
                            // Duration must be positive and reasonable (< 1 hour per chunk)
                            if d > 0.0 && d < 3600.0 {
                                Some(d)
                            } else {
                                None
                            }
                        })
                    }
                });

            // Validate that the chunk has valid video streams using ffprobe
            let has_valid_stream = std::process::Command::new(&ffprobe_cmd)
                .args(&[
                    "-v", "quiet",
                    "-select_streams", "v:0",
                    "-show_entries", "stream=codec_type",
                    "-of", "default=noprint_wrappers=1:nokey=1",
                    chunk_path.to_str().unwrap()
                ])
                .output()
                .map(|output| {
                    output.status.success() && !output.stdout.is_empty()
                })
                .unwrap_or(false);

            // Get video codec info to ensure it's actually H.264
            let has_valid_codec = std::process::Command::new(&ffprobe_cmd)
                .args(&[
                    "-v", "quiet",
                    "-select_streams", "v:0",
                    "-show_entries", "stream=codec_name",
                    "-of", "default=noprint_wrappers=1:nokey=1",
                    chunk_path.to_str().unwrap()
                ])
                .output()
                .map(|output| {
                    if output.status.success() {
                        let codec = String::from_utf8_lossy(&output.stdout);
                        let codec_name = codec.trim();
                        // Accept h264 or hevc
                        codec_name == "h264" || codec_name == "hevc"
                    } else {
                        false
                    }
                })
                .unwrap_or(false);

            // Check if file can be read without errors by ffprobe
            let is_readable = std::process::Command::new(&ffprobe_cmd)
                .args(&[
                    "-v", "error",
                    "-i", chunk_path.to_str().unwrap(),
                    "-f", "null",
                    "-"
                ])
                .output()
                .map(|output| {
                    output.status.success() && output.stderr.is_empty()
                })
                .unwrap_or(false);

            // Only include chunk if ALL validations pass
            let is_valid = has_valid_stream
                && duration_opt.is_some()
                && has_valid_codec
                && is_readable;

            if is_valid {
                let duration = duration_opt.unwrap();
                total_chunk_duration += duration;
                log::info!("Chunk {}: {:.2}s - {}", idx + 1, duration, chunk_path.file_name().unwrap_or_default().to_string_lossy());

                // Escape single quotes in the path by replacing ' with '\''
                let path_str = chunk_path.to_string_lossy().replace("'", r"'\''");
                concat_content.push_str(&format!("file '{}'\n", path_str));
                existing_chunks += 1;
            } else {
                // Detailed error reporting
                let mut reasons = Vec::new();
                if !has_valid_stream {
                    reasons.push("no video stream");
                }
                if duration_opt.is_none() {
                    reasons.push("invalid/missing duration");
                }
                if !has_valid_codec {
                    reasons.push("unsupported codec");
                }
                if !is_readable {
                    reasons.push("contains errors");
                }

                log::warn!("Skipping chunk {} ({}): {}",
                    idx + 1,
                    reasons.join(", "),
                    chunk_path.display()
                );
                invalid_chunks += 1;
            }
        } else {
            log::warn!("Skipping missing chunk file: {}", chunk_path.display());
            missing_chunks += 1;
        }
    }

    log::info!("Total duration from chunks: {:.2}s ({:.1} minutes)", total_chunk_duration, total_chunk_duration / 60.0);
    log::info!("============================");

    if existing_chunks == 0 {
        return Err(error::ScreenRecError::InvalidParameter(
            format!("No valid chunk files found for task_id: {}", task_id)
        ));
    }

    if missing_chunks > 0 || invalid_chunks > 0 {
        let mut warning_parts = Vec::new();
        if missing_chunks > 0 {
            warning_parts.push(format!("{} missing", missing_chunks));
        }
        if invalid_chunks > 0 {
            warning_parts.push(format!("{} invalid (no video streams)", invalid_chunks));
        }
        let warning_msg = warning_parts.join(", ");

        println!("⚠️  [PROGRESS] Warning: {} chunk files skipped ({}), using {} valid chunks",
                 missing_chunks + invalid_chunks, warning_msg, existing_chunks);
        log::warn!("{} chunk files skipped ({}), concatenating {} valid chunks",
                   missing_chunks + invalid_chunks, warning_msg, existing_chunks);
    }

    std::fs::write(&concat_list_path, concat_content).map_err(|e| {
        error::ScreenRecError::EncodingError(format!("Failed to write concat list: {}", e))
    })?;

    // Determine final output path
    let final_output_path = output_path.unwrap_or_else(|| output_dir.join("final.mp4"));

    // Clean up any existing output files from previous failed attempts
    if final_output_path.exists() {
        log::warn!("Removing existing output file from previous attempt: {}", final_output_path.display());
        std::fs::remove_file(&final_output_path).ok();
    }

    // Also clean up metadata files if they exist
    let metadata_path = output_dir.join("metadata.json");
    let frames_path = output_dir.join("frames.json");
    if metadata_path.exists() {
        log::warn!("Removing existing metadata.json from previous attempt");
        std::fs::remove_file(&metadata_path).ok();
    }
    if frames_path.exists() {
        log::warn!("Removing existing frames.json from previous attempt");
        std::fs::remove_file(&frames_path).ok();
    }

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
            // Frame rate params (only for re-encoding)
            "-r".to_string(), fps.to_string(),
            "-fps_mode".to_string(), "cfr".to_string(),
        ]);
    } else {
        // No normalization needed, use copy mode
        // Note: Cannot use -r or -fps_mode with -c copy as they require re-encoding
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
        log::error!("FFmpeg stderr: {}", stderr);
        return Err(error::ScreenRecError::EncodingError(format!(
            "FFmpeg concat failed: {}",
            stderr
        )));
    }

    // Clean up concat list file
    let _ = std::fs::remove_file(&concat_list_path);

    // Validate the output file was created and has valid content
    if !final_output_path.exists() {
        println!("❌ [PROGRESS] Output file was not created");
        return Err(error::ScreenRecError::EncodingError(
            "FFmpeg did not create output file".to_string()
        ));
    }

    // Check file size - if it's too small (< 1KB), it's likely corrupted
    let file_size = std::fs::metadata(&final_output_path)
        .map(|m| m.len())
        .unwrap_or(0);

    if file_size < 1024 {
        println!("❌ [PROGRESS] Output file is too small ({} bytes) - likely corrupted", file_size);
        log::error!("Output file is only {} bytes, removing corrupted file", file_size);
        std::fs::remove_file(&final_output_path).ok();
        return Err(error::ScreenRecError::EncodingError(
            format!("FFmpeg produced invalid output file ({} bytes)", file_size)
        ));
    }

    println!("✅ [PROGRESS] Video concatenation complete!");
    log::info!("✅ Final video created: {}", final_output_path.display());
    println!("✅ Final video saved to: {}", final_output_path.display());

    log::info!("===== DURATION COMPARISON =====");

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
                        log::info!("Sum of chunk durations: {:.2}s ({:.1} min)", total_chunk_duration, total_chunk_duration / 60.0);
                        log::info!("Final video duration:    {:.2}s ({:.1} min)", video_duration_secs, video_duration_secs / 60.0);
                        let diff = (video_duration_secs - total_chunk_duration).abs();
                        let diff_pct = (diff / total_chunk_duration * 100.0).abs();
                        if diff > 1.0 {
                            log::warn!("Duration mismatch: {:.2}s difference ({:.1}%)", diff, diff_pct);
                        } else {
                            log::info!("Duration match: within {:.2}s ({:.2}%)", diff, diff_pct);
                        }
                        log::info!("==============================");
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

    // Force WAL checkpoint to ensure all writes from recording sessions are visible
    log::debug!("Forcing WAL checkpoint before reading session times");
    if let Err(e) = db.checkpoint_wal().await {
        log::warn!("Failed to checkpoint WAL: {}", e);
    }

    // Get all sessions to verify they exist
    let sessions = db.get_sessions_for_task(task_id).await?;
    log::info!("Found {} recording sessions for task {}", sessions.len(), task_id);

    for session in &sessions {
        if let Some(ended_at) = session.ended_at {
            let duration = (ended_at - session.started_at).num_seconds();
            log::debug!("Session {}: duration = {}s (started: {}, ended: {})",
                       session.id, duration, session.started_at, ended_at);
        } else {
            log::warn!("Session {} has no end time (incomplete session)", session.id);
        }
    }

    // Get total recording time from database
    log::debug!("Querying total recording time for task {}", task_id);
    let total_recording_time_secs = match db.get_total_recording_time(task_id).await {
        Ok(time) => {
            log::info!("Total recording time retrieved: {:.2}s ({:.2} minutes)", time, time / 60.0);
            if time == 0.0 && !sessions.is_empty() {
                log::error!("WARNING: Database returned 0.0 seconds but {} sessions exist!", sessions.len());
                log::error!("This indicates a data integrity issue - sessions may be missing end times");
            }
            time
        }
        Err(e) => {
            log::error!("Failed to get total recording time: {}", e);
            log::error!("Defaulting to 0.0 - metadata will be incorrect!");
            0.0
        }
    };

    // Export comprehensive metadata to JSON
    println!("🔄 [PROGRESS] Generating metadata files...");
    log::info!("Exporting comprehensive metadata to JSON...");

    let metadata_output = serde_json::json!({
        "version": "1.0",
        "task_id": task_id,
        "device_name": device_name,
        "recording_type": "task",
        "created_at": chunks.first().map(|c| c.created_at.to_rfc3339()).unwrap_or_default(),
        "recording_time": {
            "total_seconds": total_recording_time_secs,
            "total_formatted": format!("{}h {}m {:.1}s",
                (total_recording_time_secs / 3600.0) as i64,
                ((total_recording_time_secs % 3600.0) / 60.0) as i64,
                total_recording_time_secs % 60.0
            ),
            "overhead_seconds": if total_recording_time_secs > video_duration_secs {
                total_recording_time_secs - video_duration_secs
            } else {
                0.0
            },
            "efficiency_percent": if total_recording_time_secs > 0.0 {
                (video_duration_secs / total_recording_time_secs) * 100.0
            } else {
                0.0
            },
        },
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

    let frames_path = output_dir.join("frames.json");
    std::fs::write(&frames_path, serde_json::to_string_pretty(&frames_output).unwrap())
        .map_err(|e| {
            error::ScreenRecError::EncodingError(format!("Failed to write frames JSON: {}", e))
        })?;

    println!("✅ [PROGRESS] Frame data exported ({} frames)", frames.len());
    log::info!("✅ Detailed frame metadata exported: {}", frames_path.display());
    println!("   📄 {}", frames_path.display());

    println!("\n🎉 [PROGRESS] Concatenation complete!");
    println!("   Video Duration: {:.1}s | Size: {:.2}MB | Frames: {}",
        video_duration_secs,
        file_size_bytes as f64 / 1024.0 / 1024.0,
        frames.len()
    );

    if total_recording_time_secs > 0.0 {
        println!("   Recording Time: {:.1}s | Overhead: {:.1}s | Efficiency: {:.1}%",
            total_recording_time_secs,
            total_recording_time_secs - video_duration_secs,
            (video_duration_secs / total_recording_time_secs) * 100.0
        );
        log::info!("===== RECORDING TIME COMPARISON =====");
        log::info!("Total recording time: {:.1}s ({:.1} min)", total_recording_time_secs, total_recording_time_secs / 60.0);
        log::info!("Video duration:       {:.1}s ({:.1} min)", video_duration_secs, video_duration_secs / 60.0);
        log::info!("Overhead:             {:.1}s", total_recording_time_secs - video_duration_secs);
        log::info!("Efficiency:           {:.1}%", (video_duration_secs / total_recording_time_secs) * 100.0);
        log::info!("====================================");
    }

    Ok(())
}
