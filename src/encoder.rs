use crate::audio::AudioSample;
use crate::capture::Frame;
use crate::error::{Result, ScreenRecError};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use tokio::sync::mpsc;
use vpx_encode::{Config, Encoder, Image};

pub struct VideoEncoder {
    encoder: Encoder,
    output_file: File,
    frame_count: u64,
    width: usize,
    height: usize,
    fps: u32,
    ivf_header_written: bool,
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
            "Initializing video encoder: {}x{} @ {}fps, quality: {}",
            width,
            height,
            fps,
            quality
        );

        // Validate quality
        let quality = quality.clamp(1, 10);

        // Create encoder config
        let mut config = Config::new(width, height, fps);

        // Configure for performance and quality
        // Lower quality value = better quality but larger file
        let target_bitrate = match quality {
            1..=3 => width * height * fps as usize / 10,  // Low quality
            4..=6 => width * height * fps as usize / 6,   // Medium quality
            7..=8 => width * height * fps as usize / 4,   // High quality
            _ => width * height * fps as usize / 3,       // Very high quality
        };

        config.set_target_bitrate(target_bitrate);
        config.set_cpu_used(6); // Balance between speed and quality (0-16, higher = faster)

        // Create encoder
        let encoder = Encoder::new(config).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to create encoder: {:?}", e))
        })?;

        // Create output file
        let output_file = File::create(output_path).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to create output file: {}", e))
        })?;

        Ok(Self {
            encoder,
            output_file,
            frame_count: 0,
            width,
            height,
            fps,
            ivf_header_written: false,
        })
    }

    fn write_ivf_header(&mut self) -> Result<()> {
        // IVF file format header
        // https://wiki.multimedia.cx/index.php/IVF
        let mut header = Vec::new();

        // File signature: "DKIF"
        header.extend_from_slice(b"DKIF");

        // Version (2 bytes): 0
        header.extend_from_slice(&[0, 0]);

        // Header size (2 bytes): 32
        header.extend_from_slice(&[32, 0]);

        // Codec FourCC (4 bytes): "VP80" for VP8
        header.extend_from_slice(b"VP80");

        // Width (2 bytes, little-endian)
        header.extend_from_slice(&(self.width as u16).to_le_bytes());

        // Height (2 bytes, little-endian)
        header.extend_from_slice(&(self.height as u16).to_le_bytes());

        // Frame rate (4 bytes, little-endian)
        header.extend_from_slice(&self.fps.to_le_bytes());

        // Time scale (4 bytes, little-endian)
        header.extend_from_slice(&1u32.to_le_bytes());

        // Number of frames (4 bytes, little-endian) - will be updated at the end
        header.extend_from_slice(&0u32.to_le_bytes());

        // Unused (4 bytes)
        header.extend_from_slice(&[0, 0, 0, 0]);

        self.output_file.write_all(&header).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to write IVF header: {}", e))
        })?;

        self.ivf_header_written = true;
        Ok(())
    }

    fn write_frame(&mut self, data: &[u8], timestamp: u64) -> Result<()> {
        // IVF frame header (12 bytes)
        let mut frame_header = Vec::new();

        // Frame size (4 bytes, little-endian)
        frame_header.extend_from_slice(&(data.len() as u32).to_le_bytes());

        // Timestamp (8 bytes, little-endian)
        frame_header.extend_from_slice(&timestamp.to_le_bytes());

        self.output_file.write_all(&frame_header).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to write frame header: {}", e))
        })?;

        self.output_file.write_all(data).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to write frame data: {}", e))
        })?;

        Ok(())
    }

    pub fn encode_frame(&mut self, frame: Frame) -> Result<()> {
        // Write IVF header on first frame
        if !self.ivf_header_written {
            self.write_ivf_header()?;
        }

        // Create image for encoder
        let image = Image::from_rgb_bytes(self.width, self.height, &frame.data).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to create image: {:?}", e))
        })?;

        // Encode frame
        let packets = self
            .encoder
            .encode(self.frame_count as i64, &image)
            .map_err(|e| {
                ScreenRecError::EncodingError(format!("Failed to encode frame: {:?}", e))
            })?;

        // Write encoded packets
        for packet in packets {
            self.write_frame(packet.data, self.frame_count)?;
        }

        self.frame_count += 1;

        if self.frame_count % (self.fps as u64) == 0 {
            log::debug!("Encoded {} frames", self.frame_count);
        }

        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        log::info!("Finishing encoding, total frames: {}", self.frame_count);

        // Flush remaining frames
        loop {
            match self.encoder.flush() {
                Ok(packets) => {
                    if packets.is_empty() {
                        break;
                    }
                    for packet in packets {
                        self.write_frame(packet.data, self.frame_count)?;
                        self.frame_count += 1;
                    }
                }
                Err(e) => {
                    log::warn!("Flush error: {:?}", e);
                    break;
                }
            }
        }

        // Update frame count in header
        use std::io::Seek;
        self.output_file.seek(std::io::SeekFrom::Start(24)).map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to seek to frame count: {}", e))
        })?;

        self.output_file
            .write_all(&(self.frame_count as u32).to_le_bytes())
            .map_err(|e| {
                ScreenRecError::EncodingError(format!("Failed to update frame count: {}", e))
            })?;

        self.output_file.flush().map_err(|e| {
            ScreenRecError::EncodingError(format!("Failed to flush output file: {}", e))
        })?;

        log::info!("Encoding finished successfully");
        Ok(())
    }
}

/// Process frames from the capture channel and encode them
pub async fn process_frames(
    mut rx: mpsc::Receiver<Frame>,
    mut encoder: VideoEncoder,
) -> Result<()> {
    log::info!("Starting frame processing");

    while let Some(frame) = rx.recv().await {
        encoder.encode_frame(frame)?;
    }

    encoder.finish()?;
    Ok(())
}

/// Process audio samples (currently just logs them, can be extended to mux with video)
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
