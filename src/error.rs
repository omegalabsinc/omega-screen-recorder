use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScreenRecError {
    #[error("Screen capture error: {0}")]
    CaptureError(String),

    #[error("Audio capture error: {0}")]
    AudioError(String),

    #[error("Encoding error: {0}")]
    EncodingError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Image error: {0}")]
    ImageError(#[from] image::ImageError),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[error("Platform not supported: {0}")]
    #[allow(dead_code)]
    PlatformNotSupported(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Encoder busy or in use: {0}")]
    #[allow(dead_code)]
    EncoderBusy(String),

    #[error("Hardware encoder unavailable: {0}")]
    #[allow(dead_code)]
    HardwareEncoderUnavailable(String),

    #[error("Encoder initialization failed after {1} retries: {0}")]
    #[allow(dead_code)]
    EncoderInitializationFailed(String, u32),

    #[error("Audio device unavailable, tried: {0:?}")]
    AudioDeviceUnavailable(Vec<String>),

    #[error("Encoder failure during recording: {0}")]
    EncoderRuntimeFailure(String),
}

impl From<anyhow::Error> for ScreenRecError {
    fn from(err: anyhow::Error) -> Self {
        ScreenRecError::DatabaseError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ScreenRecError>;
