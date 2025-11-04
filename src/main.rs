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
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse command line arguments
    let cli = Cli::parse();

    // Initialize logger
    let log_level = if cli.verbose {
        "debug"
    } else {
        "info"
    };
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
            no_skip_idle,
            track_interactions,
            track_mouse_moves,
        } => {
            log::info!("Starting screen recording...");
            log::info!("  Output: {}", output.display());
            log::info!("  FPS: {}", fps);
            log::info!("  Duration: {}", if duration > 0 {
                format!("{} seconds", duration)
            } else {
                "unlimited (Ctrl+C to stop)".to_string()
            });
            log::info!("  Audio: {}", audio);
            log::info!("  Quality: {}/10", quality);
            log::info!("  Idle frame skipping: {}", if no_skip_idle { "disabled" } else { "enabled" });
            log::info!("  Interaction tracking: {}", if track_interactions { "enabled" } else { "disabled" });

            // Validate FPS
            if fps == 0 || fps > 60 {
                return Err(error::ScreenRecError::InvalidParameter(
                    "FPS must be between 1 and 60".to_string(),
                ));
            }

            // Initialize screen capture
            let screen_capture = ScreenCapture::new(display, fps)?;
            let capture_width = if width > 0 { width as usize } else { screen_capture.width() };
            let capture_height = if height > 0 { height as usize } else { screen_capture.height() };

            log::info!("Capture resolution: {}x{}", capture_width, capture_height);

            // Create channels for frame and audio data
            let (frame_tx, frame_rx) = mpsc::channel(60); // Buffer up to 2 seconds of frames at 30fps
            let (audio_tx, audio_rx) = mpsc::channel(1000);

            // Initialize video encoder
            let encoder = VideoEncoder::new(&output, capture_width, capture_height, fps, quality)?;

            // Start encoder task
            let encoder_handle = tokio::spawn(async move {
                encoder::process_frames(frame_rx, encoder).await
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

            // Initialize interaction tracker if requested
            let interaction_tracker = if track_interactions {
                let tracker = InteractionTracker::new(
                    capture_width,
                    capture_height,
                    track_mouse_moves,
                );

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

            // Start screen capture
            let duration_opt = if duration > 0 {
                Some(Duration::from_secs(duration))
            } else {
                None
            };

            let skip_idle_frames = !no_skip_idle; // Invert because CLI flag disables the feature
            let capture_handle = tokio::spawn(async move {
                screen_capture.start_capture(frame_tx, duration_opt, skip_idle_frames).await
            });

            // Wait for capture to finish
            let capture_result = capture_handle.await.map_err(|e| {
                error::ScreenRecError::CaptureError(format!("Capture task failed: {}", e))
            })??;

            // Wait for encoder to finish
            let encoder_result = encoder_handle.await.map_err(|e| {
                error::ScreenRecError::EncodingError(format!("Encoder task failed: {}", e))
            })??;

            // Wait for audio processing if it was started
            if let Some(handle) = audio_handle {
                let _ = handle.await;
            }

            // Save interaction data if tracking was enabled
            if let Some((tracker, _handle)) = interaction_tracker {
                // Create interaction data file path
                let interactions_path = output.with_extension("interactions.json");

                log::info!("Saving interaction data...");
                if let Err(e) = tracker.save(&interactions_path) {
                    log::error!("Failed to save interaction data: {}", e);
                } else {
                    println!("✅ Interactions saved to: {}", interactions_path.display());
                }
            }

            println!("✅ Recording saved to: {}", output.display());
            log::info!("Recording completed successfully");
        }
    }

    Ok(())
}
