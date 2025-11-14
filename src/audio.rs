use crate::cli::AudioSource;
use crate::error::{Result, ScreenRecError};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use tokio::sync::mpsc;

#[allow(dead_code)]
pub struct AudioSample {
    pub data: Vec<f32>,
    pub sample_rate: u32,
}

pub struct AudioCapture {
    device: Device,
    config: StreamConfig,
}

impl AudioCapture {
    pub fn new(source: AudioSource) -> Result<Option<Self>> {
        if source == AudioSource::None {
            log::info!("Audio capture disabled");
            return Ok(None);
        }

        log::info!("Initializing audio capture with source: {:?}", source);

        let host = cpal::default_host();

        // Get the appropriate device based on source
        let device = match source {
            AudioSource::None => return Ok(None),
            AudioSource::Mic | AudioSource::Both => {
                // For microphone, use default input device
                host.default_input_device().ok_or_else(|| {
                    ScreenRecError::AudioError("No input device found".to_string())
                })?
            }
            AudioSource::System => {
                // For system audio, try to get loopback device
                // Note: System audio capture is tricky on some platforms
                host.default_input_device().ok_or_else(|| {
                    ScreenRecError::AudioError("No input device found".to_string())
                })?
            }
        };

        let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        log::info!("Using audio device: {}", device_name);

        // Get supported config
        let config = device
            .default_input_config()
            .map_err(|e| ScreenRecError::AudioError(format!("Failed to get audio config: {}", e)))?
            .config();

        log::info!("Audio config: {:?}", config);

        Ok(Some(Self { device, config }))
    }

    #[allow(dead_code)]
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate.0
    }

    #[allow(dead_code)]
    pub fn channels(&self) -> u16 {
        self.config.channels
    }

    /// Start capturing audio and send samples through the channel
    pub fn start_capture(self, tx: mpsc::Sender<AudioSample>) -> Result<Stream> {
        let sample_rate = self.config.sample_rate.0;
        let channels = self.config.channels;

        log::info!(
            "Starting audio capture at {} Hz, {} channels",
            sample_rate,
            channels
        );

        let stream = self
            .device
            .build_input_stream(
                &self.config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Convert to mono if stereo
                    let mono_data: Vec<f32> = if channels == 2 {
                        data.chunks_exact(2)
                            .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
                            .collect()
                    } else {
                        data.to_vec()
                    };

                    let sample = AudioSample {
                        data: mono_data,
                        sample_rate,
                    };

                    // Try to send, but don't block if receiver is slow
                    let _ = tx.try_send(sample);
                },
                |err| {
                    log::error!("Audio stream error: {}", err);
                },
                None,
            )
            .map_err(|e| {
                ScreenRecError::AudioError(format!("Failed to build audio stream: {}", e))
            })?;

        stream.play().map_err(|e| {
            ScreenRecError::AudioError(format!("Failed to play audio stream: {}", e))
        })?;

        Ok(stream)
    }
}
