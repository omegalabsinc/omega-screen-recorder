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
}

impl From<anyhow::Error> for ScreenRecError {
    fn from(err: anyhow::Error) -> Self {
        ScreenRecError::DatabaseError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ScreenRecError>;
