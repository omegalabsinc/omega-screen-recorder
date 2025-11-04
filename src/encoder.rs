use crate::audio::AudioSample;
use crate::capture::Frame;
use crate::error::{Result, ScreenRecError};
use image::{ImageBuffer, RgbImage};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct VideoEncoder {
    output_dir: PathBuf,
    frame_count: u64,
    width: usize,
    height: usize,
    fps: u32,
    manifest_file: File,
}

impl VideoEncoder {
    pub fn new(
        output_path: &Path,
        width: usize,
        height: usize,
        fps: u32,
        _quality: u8,
    ) -> Result<Self> {
        log::info!(
            "Initializing frame storage: {}x{} @ {}fps",
            width,
            height,
            fps
        );

        // Create output directory for frames
        let output_dir = output_path.with_extension("frames");
        fs::create_dir_all(&output_dir)?;

        // Create manifest file
        let manifest_path = output_dir.join("manifest.txt");
        let mut manifest_file = File::create(&manifest_path)?;

        // Write manifest header
        writeln!(manifest_file, "# Screen Recording Manifest")?;
        writeln!(manifest_file, "width={}", width)?;
        writeln!(manifest_file, "height={}", height)?;
        writeln!(manifest_file, "fps={}", fps)?;
        writeln!(manifest_file, "format=jpeg")?;
        writeln!(manifest_file, "# Frame list (one per line):")?;

        log::info!("Saving frames to: {:?}", output_dir);

        Ok(Self {
            output_dir,
            frame_count: 0,
            width,
            height,
            fps,
            manifest_file,
        })
    }

    pub fn encode_frame(&mut self, frame: Frame) -> Result<()> {
        // Create image from RGB data
        let img: RgbImage = ImageBuffer::from_raw(
            self.width as u32,
            self.height as u32,
            frame.data,
        )
        .ok_or_else(|| ScreenRecError::EncodingError("Failed to create image buffer".to_string()))?;

        // Save frame as JPEG
        let frame_filename = format!("frame_{:08}.jpg", self.frame_count);
        let frame_path = self.output_dir.join(&frame_filename);

        img.save_with_format(&frame_path, image::ImageFormat::Jpeg)
            .map_err(|e| ScreenRecError::EncodingError(format!("Failed to save frame: {}", e)))?;

        // Write to manifest
        writeln!(self.manifest_file, "{}", frame_filename)?;

        self.frame_count += 1;

        if self.frame_count % (self.fps as u64) == 0 {
            log::debug!("Saved {} frames", self.frame_count);
        }

        Ok(())
    }

    pub fn finish(mut self) -> Result<PathBuf> {
        log::info!("Finishing encoding, total frames: {}", self.frame_count);

        // Finalize manifest
        writeln!(self.manifest_file, "# Total frames: {}", self.frame_count)?;
        self.manifest_file.flush()?;

        // Create conversion script
        self.create_conversion_script()?;

        log::info!("Frames saved to: {:?}", self.output_dir);
        log::info!("Run convert.sh (Mac/Linux) or convert.bat (Windows) to create video");

        Ok(self.output_dir)
    }

    fn create_conversion_script(&self) -> Result<()> {
        // Create bash script for Mac/Linux
        let bash_script = format!(
            r#"#!/bin/bash
# Screen Recording Conversion Script
# This script converts the captured frames to a video file

OUTPUT_FILE="../recording.mp4"
FPS={}
WIDTH={}
HEIGHT={}

echo "Converting frames to video..."
echo "Output: $OUTPUT_FILE"

# Check if ffmpeg is installed
if ! command -v ffmpeg &> /dev/null; then
    echo "Error: ffmpeg is not installed"
    echo "Install with: brew install ffmpeg (macOS) or apt-get install ffmpeg (Linux)"
    exit 1
fi

# Convert frames to video
ffmpeg -framerate $FPS -pattern_type glob -i 'frame_*.jpg' \
    -c:v libx264 -preset medium -crf 23 \
    -pix_fmt yuv420p -s ${{WIDTH}}x${{HEIGHT}} \
    "$OUTPUT_FILE" -y

if [ $? -eq 0 ]; then
    echo "✅ Video created: $OUTPUT_FILE"
    echo "Duration: $(ffprobe -v error -show_entries format=duration -of default=noprint_wrappers=1:nokey=1 "$OUTPUT_FILE") seconds"
else
    echo "❌ Conversion failed"
    exit 1
fi
"#,
            self.fps, self.width, self.height
        );

        let bash_path = self.output_dir.join("convert.sh");
        let mut bash_file = File::create(&bash_path)?;
        bash_file.write_all(bash_script.as_bytes())?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bash_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bash_path, perms)?;
        }

        // Create batch script for Windows
        let bat_script = format!(
            r#"@echo off
REM Screen Recording Conversion Script
REM This script converts the captured frames to a video file

set OUTPUT_FILE=..\recording.mp4
set FPS={}
set WIDTH={}
set HEIGHT={}

echo Converting frames to video...
echo Output: %OUTPUT_FILE%

REM Check if ffmpeg is installed
where ffmpeg >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo Error: ffmpeg is not installed
    echo Download from: https://ffmpeg.org/download.html
    exit /b 1
)

REM Convert frames to video
ffmpeg -framerate %FPS% -i frame_%%08d.jpg ^
    -c:v libx264 -preset medium -crf 23 ^
    -pix_fmt yuv420p -s %WIDTH%x%HEIGHT% ^
    "%OUTPUT_FILE%" -y

if %ERRORLEVEL% EQU 0 (
    echo Video created: %OUTPUT_FILE%
) else (
    echo Conversion failed
    exit /b 1
)
"#,
            self.fps, self.width, self.height
        );

        let bat_path = self.output_dir.join("convert.bat");
        let mut bat_file = File::create(&bat_path)?;
        bat_file.write_all(bat_script.as_bytes())?;

        log::info!("Created conversion scripts: convert.sh and convert.bat");

        Ok(())
    }
}

/// Process frames from the capture channel and encode them
pub async fn process_frames(
    mut rx: mpsc::Receiver<Frame>,
    mut encoder: VideoEncoder,
) -> Result<PathBuf> {
    log::info!("Starting frame processing");

    while let Some(frame) = rx.recv().await {
        encoder.encode_frame(frame)?;
    }

    encoder.finish()
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
