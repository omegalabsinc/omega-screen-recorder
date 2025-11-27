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

/// Enumerate audio devices based on source type
fn enumerate_audio_devices(
    host: &cpal::Host,
    source: &AudioSource,
) -> Result<Vec<(Device, String)>> {
    let mut devices = Vec::new();

    match source {
        AudioSource::None => return Ok(devices),
        AudioSource::Mic | AudioSource::Both => {
            // Try default input device first
            if let Some(device) = host.default_input_device() {
                let name = device.name().unwrap_or_else(|_| "Default Input".to_string());
                log::debug!("Found default input device: {}", name);
                devices.push((device, name));
            }

            // Enumerate all input devices
            if let Ok(input_devices) = host.input_devices() {
                for device in input_devices {
                    if let Ok(name) = device.name() {
                        if !devices.iter().any(|(_, n)| n == &name) {
                            log::debug!("Found input device: {}", name);
                            devices.push((device, name));
                        }
                    }
                }
            }
        }
        AudioSource::System => {
            // Platform-specific system audio device detection
            #[cfg(target_os = "macos")]
            {
                if let Ok(devices_iter) = host.input_devices() {
                    for device in devices_iter {
                        if let Ok(name) = device.name() {
                            if name.contains("Soundflower") ||
                               name.contains("BlackHole") ||
                               name.contains("Loopback") {
                                log::debug!("Found system audio device: {}", name);
                                devices.push((device, name));
                            }
                        }
                    }
                }
            }

            #[cfg(target_os = "windows")]
            {
                if let Ok(devices_iter) = host.input_devices() {
                    for device in devices_iter {
                        if let Ok(name) = device.name() {
                            if name.contains("Stereo Mix") ||
                               name.contains("What U Hear") {
                                log::debug!("Found system audio device: {}", name);
                                devices.push((device, name));
                            }
                        }
                    }
                }
            }

            // Fallback to default input if no system audio device found
            if devices.is_empty() {
                log::warn!("No system audio device found, using default input");
                if let Some(device) = host.default_input_device() {
                    let name = device.name().unwrap_or_else(|_| "Default Input".to_string());
                    devices.push((device, name));
                }
            }
        }
    }

    Ok(devices)
}

impl AudioCapture {
    pub fn new(source: AudioSource) -> Result<Option<Self>> {
        if source == AudioSource::None {
            log::info!("Audio capture disabled");
            return Ok(None);
        }

        log::info!("Initializing audio capture with source: {:?}", source);

        let host = cpal::default_host();

        // Enumerate available devices
        let available_devices = enumerate_audio_devices(&host, &source)?;

        if available_devices.is_empty() {
            log::warn!("No audio devices found for source: {:?}", source);
            return Err(ScreenRecError::AudioDeviceUnavailable(vec![]));
        }

        log::info!("Found {} audio device(s)", available_devices.len());

        // Try devices in order
        let mut tried_devices = Vec::new();

        for (device, device_name) in available_devices {
            log::info!("Trying audio device: {}", device_name);
            tried_devices.push(device_name.clone());

            match device.default_input_config() {
                Ok(config) => {
                    log::info!("✓ Successfully initialized audio device: {}", device_name);
                    return Ok(Some(Self { device, config: config.config() }));
                }
                Err(e) => {
                    log::warn!("Failed to initialize device '{}': {}", device_name, e);
                    continue;
                }
            }
        }

        // All devices failed
        log::warn!("All audio devices failed. Tried: {:?}", tried_devices);
        Err(ScreenRecError::AudioDeviceUnavailable(tried_devices))
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
