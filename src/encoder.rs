use crate::audio::AudioSample;
use crate::capture::Frame;
use crate::error::{Result, ScreenRecError};
use image::{ImageBuffer, RgbImage};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct RecordingOutput {
    pub frames_dir: PathBuf,
    pub video_file: Option<PathBuf>,
}

struct FrameEntry {
    filename: String,
    timestamp: Duration,
}

pub struct VideoEncoder {
    base_output: PathBuf,
    output_dir: PathBuf,
    frame_entries: Vec<FrameEntry>,
    frame_count: u64,
    width: usize,
    height: usize,
    fps: u32,
    crf: u8,
    manifest_file: File,
}

impl VideoEncoder {
    pub fn new(
        output_path: &Path,
        width: usize,
        height: usize,
        fps: u32,
        quality: u8,
    ) -> Result<Self> {
        log::info!(
            "Initializing frame storage: {}x{} @ {}fps",
            width,
            height,
            fps
        );

        let crf = Self::quality_to_crf(quality);
        let base_output = output_path.to_path_buf();

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
        writeln!(manifest_file, "# Frame format: filename,timestamp_ms")?;

        log::info!("Saving frames to: {:?}", output_dir);

        Ok(Self {
            base_output,
            output_dir,
            frame_entries: Vec::new(),
            frame_count: 0,
            width,
            height,
            fps,
            crf,
            manifest_file,
        })
    }

    pub fn encode_frame(&mut self, frame: Frame) -> Result<()> {
        let Frame {
            data, timestamp, ..
        } = frame;

        let img: RgbImage = ImageBuffer::from_raw(self.width as u32, self.height as u32, data)
            .ok_or_else(|| {
                ScreenRecError::EncodingError("Failed to create image buffer".to_string())
            })?;

        let frame_filename = format!("frame_{:08}.jpg", self.frame_count);
        let frame_path = self.output_dir.join(&frame_filename);

        img.save_with_format(&frame_path, image::ImageFormat::Jpeg)
            .map_err(|e| ScreenRecError::EncodingError(format!("Failed to save frame: {}", e)))?;

        self.frame_entries.push(FrameEntry {
            filename: frame_filename.clone(),
            timestamp,
        });

        writeln!(
            self.manifest_file,
            "{},{}",
            frame_filename,
            timestamp.as_millis()
        )?;

        self.frame_count += 1;

        if self.frame_count % (self.fps as u64) == 0 {
            log::debug!("Saved {} frames", self.frame_count);
        }

        Ok(())
    }

    pub fn finish(mut self) -> Result<RecordingOutput> {
        log::info!("Finishing encoding, total frames: {}", self.frame_count);

        writeln!(self.manifest_file, "# Total frames: {}", self.frame_count)?;
        self.manifest_file.flush()?;

        if self.frame_entries.is_empty() {
            log::warn!("No frames captured; skipping video generation");
            return Ok(RecordingOutput {
                frames_dir: self.output_dir,
                video_file: None,
            });
        }

        let concat_path = self.write_concat_file()?;
        self.create_conversion_script()?;
        let video_file = self.try_generate_video(&concat_path)?;

        log::info!("Frames saved to: {}", self.output_dir.display());
        if let Some(ref video_path) = video_file {
            log::info!("Video saved to: {}", video_path.display());
        } else {
            log::info!("FFmpeg not found or failed; use the generated convert script to build the video manually.");
        }

        Ok(RecordingOutput {
            frames_dir: self.output_dir,
            video_file,
        })
    }

    fn write_concat_file(&self) -> Result<PathBuf> {
        let concat_path = self.output_dir.join("frames.ffconcat");
        let mut concat_file = File::create(&concat_path)?;

        writeln!(concat_file, "ffconcat version 1.0")?;

        if self.frame_entries.len() == 1 {
            let entry = &self.frame_entries[0];
            let duration = self.fallback_frame_duration();

            writeln!(concat_file, "file {}", entry.filename)?;
            writeln!(concat_file, "duration {:.6}", duration)?;
            writeln!(concat_file, "file {}", entry.filename)?;
        } else {
            for idx in 0..(self.frame_entries.len() - 1) {
                let entry = &self.frame_entries[idx];
                let next = &self.frame_entries[idx + 1];
                let duration = self.duration_between(entry.timestamp, next.timestamp);

                writeln!(concat_file, "file {}", entry.filename)?;
                writeln!(concat_file, "duration {:.6}", duration)?;
            }

            let last_entry = self.frame_entries.last().expect("entries not empty");
            let last_duration = if self.frame_entries.len() >= 2 {
                let prev = &self.frame_entries[self.frame_entries.len() - 2];
                self.duration_between(prev.timestamp, last_entry.timestamp)
            } else {
                self.fallback_frame_duration()
            };

            writeln!(concat_file, "file {}", last_entry.filename)?;
            writeln!(concat_file, "duration {:.6}", last_duration)?;
            writeln!(concat_file, "file {}", last_entry.filename)?;
        }

        concat_file.flush()?;
        Ok(concat_path)
    }

    fn create_conversion_script(&self) -> Result<()> {
        let target_path = self.target_video_path();
        let video_filename = target_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("recording.mp4");

        // Create bash script for Mac/Linux
        let bash_script = format!(
            r#"#!/bin/bash
# Screen Recording Conversion Script
# This script converts the captured frames to a video file

OUTPUT_FILE="../{video_name}"
CRF={crf}
FFCONCAT_FILE="frames.ffconcat"

if [ ! -f "$FFCONCAT_FILE" ]; then
    echo "Error: $FFCONCAT_FILE not found"
    exit 1
fi

echo "Converting frames to video..."
echo "Output: $OUTPUT_FILE"

# Check if ffmpeg is installed
if ! command -v ffmpeg &> /dev/null; then
    echo "Error: ffmpeg is not installed"
    echo "Install with: brew install ffmpeg (macOS) or apt-get install ffmpeg (Linux)"
    exit 1
fi

ffmpeg -y -f concat -safe 0 -i "$FFCONCAT_FILE" \
    -vsync vfr -pix_fmt yuv420p \
    -c:v libx264 -preset medium -crf $CRF \
    "$OUTPUT_FILE"

if [ $? -eq 0 ]; then
    echo "✅ Video created: $OUTPUT_FILE"
else
    echo "❌ Conversion failed"
    exit 1
fi
"#,
            video_name = video_filename,
            crf = self.crf,
        );

        let bash_path = self.output_dir.join("convert.sh");
        let mut bash_file = File::create(&bash_path)?;
        bash_file.write_all(bash_script.as_bytes())?;

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

set OUTPUT_FILE=..\{video_name}
set CRF={crf}
set FFCONCAT_FILE=frames.ffconcat

if not exist %FFCONCAT_FILE% (
    echo Error: %FFCONCAT_FILE% not found
    exit /b 1
)

echo Converting frames to video...
echo Output: %OUTPUT_FILE%

where ffmpeg >nul 2>nul
if %ERRORLEVEL% NEQ 0 (
    echo Error: ffmpeg is not installed
    echo Download from: https://ffmpeg.org/download.html
    exit /b 1
)

ffmpeg -y -f concat -safe 0 -i %FFCONCAT_FILE% ^
    -vsync vfr -pix_fmt yuv420p ^
    -c:v libx264 -preset medium -crf %CRF% ^
    "%OUTPUT_FILE%"

if %ERRORLEVEL% EQU 0 (
    echo Video created: %OUTPUT_FILE%
) else (
    echo Conversion failed
    exit /b 1
)
"#,
            video_name = video_filename,
            crf = self.crf,
        );

        let bat_path = self.output_dir.join("convert.bat");
        let mut bat_file = File::create(&bat_path)?;
        bat_file.write_all(bat_script.as_bytes())?;

        log::info!("Created conversion scripts: convert.sh and convert.bat");
        Ok(())
    }

    fn try_generate_video(&self, concat_path: &Path) -> Result<Option<PathBuf>> {
        let output_path = self.target_video_path();
        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let status = Command::new("ffmpeg")
            .arg("-y")
            .arg("-loglevel")
            .arg("warning")
            .arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(concat_path)
            .arg("-vsync")
            .arg("vfr")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("medium")
            .arg("-crf")
            .arg(format!("{}", self.crf))
            .arg(&output_path)
            .status();

        match status {
            Ok(exit) if exit.success() => Ok(Some(output_path)),
            Ok(exit) => {
                log::error!("ffmpeg exited with status {}", exit);
                Ok(None)
            }
            Err(err) => {
                log::warn!("Unable to launch ffmpeg automatically: {}", err);
                Ok(None)
            }
        }
    }

    fn target_video_path(&self) -> PathBuf {
        if self.base_output.extension().is_some() {
            self.base_output.clone()
        } else {
            let mut path = self.base_output.clone();
            path.set_extension("mp4");
            path
        }
    }

    fn duration_between(&self, start: Duration, end: Duration) -> f64 {
        if end > start {
            let secs = (end - start).as_secs_f64();
            if secs > 0.0 {
                secs
            } else {
                self.fallback_frame_duration()
            }
        } else {
            self.fallback_frame_duration()
        }
    }

    fn fallback_frame_duration(&self) -> f64 {
        if self.fps == 0 {
            1.0 / 30.0
        } else {
            1.0 / self.fps as f64
        }
    }

    fn quality_to_crf(quality: u8) -> u8 {
        let q = quality.clamp(1, 10) as i32;
        let mapped = 42 - q * 3;
        mapped.clamp(12, 35) as u8
    }
}

/// Process frames from the capture channel and encode them
pub async fn process_frames(
    mut rx: mpsc::Receiver<Frame>,
    mut encoder: VideoEncoder,
) -> Result<RecordingOutput> {
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
