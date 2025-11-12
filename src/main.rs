mod audio;
mod capture;
mod cli;
mod encoder;
mod error;
mod interactions;
mod screenshot;

use crate::audio::AudioCapture;
use crate::capture::ScreenCapture;
use crate::cli::{Cli, Commands};
use crate::encoder::VideoEncoder;
use crate::error::Result;
use crate::interactions::InteractionTracker;
use clap::Parser;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
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

        Commands::Record {
            output,
            duration,
            fps,
            audio,
            width,
            height,
            display,
            quality,
            track_interactions,
            track_mouse_moves,
        } => {
            log::info!("Starting screen recording...");
            log::info!("  Output: {}", output.display());
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

            // Validate FPS
            if fps == 0 || fps > 60 {
                return Err(error::ScreenRecError::InvalidParameter(
                    "FPS must be between 1 and 60".to_string(),
                ));
            }

            // Initialize screen capture
            let screen_capture = ScreenCapture::new(display, fps)?;
            let capture_width = if width > 0 {
                width as usize
            } else {
                screen_capture.width()
            };
            let capture_height = if height > 0 {
                height as usize
            } else {
                screen_capture.height()
            };

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

            // Initialize video encoder
            let encoder = VideoEncoder::new(&output, capture_width, capture_height, fps, quality)?;

            // Start encoder task
            let encoder_handle =
                tokio::spawn(async move { encoder::process_frames(frame_rx, encoder).await });

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

            // Initialize interaction tracker if requested
            let interaction_tracker = if track_interactions {
                let tracker =
                    InteractionTracker::new(capture_width, capture_height, track_mouse_moves);

                // Start tracking in a separate thread
                let tracker_handle = tracker.start()?;

                Some((tracker, tracker_handle))
            } else {
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
            let capture_handle = std::thread::spawn(move || {
                screen_capture.start_capture_sync(frame_tx_std, target_frames)
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

            // Wait for encoder to finish and get video file
            let artifacts = encoder_handle.await.map_err(|e| {
                error::ScreenRecError::EncodingError(format!("Encoder task failed: {}", e))
            })??;
            let encoder::RecordingOutput { video_file } = artifacts;

            // Wait for audio processing if it was started
            if let Some(handle) = audio_handle {
                let _ = handle.await;
            }

            // Save interaction data if tracking was enabled
            if let Some((tracker, _handle)) = interaction_tracker {
                let interactions_path = if let Some(parent) = video_file.parent() {
                    parent.join("interactions.json")
                } else {
                    std::path::PathBuf::from("interactions.json")
                };

                log::info!("Saving interaction data...");
                if let Err(e) = tracker.save(&interactions_path) {
                    log::error!("Failed to save interaction data: {}", e);
                } else {
                    println!("✅ Interactions saved to: {}", interactions_path.display());
                }
            }

            println!("✅ Video saved to: {}", video_file.display());
            log::info!("Recording completed successfully");
        }
    }

    Ok(())
}
