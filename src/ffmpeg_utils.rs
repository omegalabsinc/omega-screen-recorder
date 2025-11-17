use crate::error::{Result, ScreenRecError};
use std::path::PathBuf;
use std::process::Command;

/// Finds the FFmpeg binary, either from the provided path or from the system PATH
pub fn find_ffmpeg_binary(provided_path: Option<&PathBuf>) -> Result<String> {
    // If a path was provided, use it
    if let Some(path) = provided_path {
        let path_str = path
            .to_str()
            .ok_or_else(|| ScreenRecError::ConfigError("Invalid FFmpeg path".to_string()))?;

        // Verify the provided path exists and is executable
        if !path.exists() {
            return Err(ScreenRecError::ConfigError(format!(
                "FFmpeg binary not found at provided path: {}",
                path_str
            )));
        }

        log::info!("Using provided FFmpeg binary: {}", path_str);
        return Ok(path_str.to_string());
    }

    // Try to find FFmpeg in system PATH
    log::info!("No FFmpeg path provided, searching system PATH...");

    match which_ffmpeg() {
        Some(path) => {
            log::info!("Found FFmpeg in system: {}", path);
            Ok(path)
        }
        None => {
            Err(ScreenRecError::ConfigError(
                "FFmpeg not found. Please install FFmpeg or provide --ffmpeg-path argument.\n\
                 \n\
                 Installation instructions:\n\
                   macOS:    brew install ffmpeg\n\
                   Ubuntu:   sudo apt-get install ffmpeg\n\
                   Windows:  Download from https://ffmpeg.org/download.html"
                    .to_string(),
            ))
        }
    }
}

/// Finds ffprobe binary based on the ffmpeg binary location
pub fn find_ffprobe_binary(ffmpeg_path: &str) -> String {
    let ffmpeg_path_buf = PathBuf::from(ffmpeg_path);

    // If ffmpeg is a full path, try to find ffprobe in the same directory
    if let Some(parent) = ffmpeg_path_buf.parent() {
        let ffprobe_path = parent.join("ffprobe");
        if ffprobe_path.exists() {
            if let Some(path_str) = ffprobe_path.to_str() {
                log::info!("Found ffprobe at: {}", path_str);
                return path_str.to_string();
            }
        }
    }

    // Otherwise, try system PATH
    if let Some(path) = which_ffprobe() {
        log::info!("Found ffprobe in system: {}", path);
        return path;
    }

    // Fallback to just "ffprobe" and hope it's in PATH
    log::warn!("Could not find ffprobe, using 'ffprobe' and hoping it's in PATH");
    "ffprobe".to_string()
}

/// Tries to find ffmpeg in the system PATH
fn which_ffmpeg() -> Option<String> {
    which_command("ffmpeg")
}

/// Tries to find ffprobe in the system PATH
fn which_ffprobe() -> Option<String> {
    which_command("ffprobe")
}

/// Generic function to find a command in system PATH
fn which_command(command: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    let which_cmd = "where";
    #[cfg(not(target_os = "windows"))]
    let which_cmd = "which";

    Command::new(which_cmd)
        .arg(command)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().lines().next().unwrap_or("").to_string())
                    .filter(|s| !s.is_empty())
            } else {
                None
            }
        })
}

/// Validates that FFmpeg is working by running -version
pub fn validate_ffmpeg(ffmpeg_path: &str) -> Result<String> {
    log::info!("Validating FFmpeg installation...");

    match Command::new(ffmpeg_path).arg("-version").output() {
        Ok(output) => {
            if output.status.success() {
                let version_info = String::from_utf8_lossy(&output.stdout);
                let first_line = version_info
                    .lines()
                    .next()
                    .unwrap_or("Unknown version")
                    .to_string();

                log::info!("FFmpeg validation successful: {}", first_line);
                Ok(first_line)
            } else {
                Err(ScreenRecError::ConfigError(format!(
                    "FFmpeg validation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }
        Err(e) => Err(ScreenRecError::ConfigError(format!(
            "Failed to execute FFmpeg: {}",
            e
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_which_command() {
        // This should work on most systems
        let result = which_command("ls");
        assert!(result.is_some());
    }
}
