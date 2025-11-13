use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "screenrec")]
#[command(author = "Omega Labs")]
#[command(version = "0.1.0")]
#[command(about = "High-performance cross-platform screen recording CLI tool", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Capture a screenshot
    Screenshot {
        /// Output file path (supports .png, .jpg, .jpeg)
        #[arg(short, long, default_value = "screenshot.png")]
        output: PathBuf,

        /// Display to capture (0 for primary display)
        #[arg(short, long, default_value = "0")]
        display: usize,
    },

    /// Record screen video with audio
    Record {
        /// Output directory name (frames will be saved here)
        #[arg(short, long, default_value = "recording")]
        output: PathBuf,

        /// Recording duration in seconds (0 for manual stop)
        #[arg(short, long, default_value = "0")]
        duration: u64,

        /// Frames per second
        #[arg(short, long, default_value = "30")]
        fps: u32,

        /// Audio source: none, system, mic, or both
        #[arg(short, long, default_value = "system")]
        audio: AudioSource,

        /// Video width (0 for screen resolution)
        #[arg(long, default_value = "0")]
        width: u32,

        /// Video height (0 for screen resolution)
        #[arg(long, default_value = "0")]
        height: u32,

        /// Display to capture (0 for primary display)
        #[arg(long, default_value = "0")]
        display: usize,

        /// Video quality (1-10, higher is better)
        #[arg(short, long, default_value = "8")]
        quality: u8,

        /// Track mouse and keyboard interactions
        #[arg(long)]
        track_interactions: bool,

        /// Track mouse movements (generates more data, only with --track-interactions)
        #[arg(long)]
        track_mouse_moves: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    None,
    System,
    Mic,
    Both,
}

impl std::str::FromStr for AudioSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(AudioSource::None),
            "system" => Ok(AudioSource::System),
            "mic" => Ok(AudioSource::Mic),
            "both" => Ok(AudioSource::Both),
            _ => Err(format!(
                "Invalid audio source: {}. Use: none, system, mic, or both",
                s
            )),
        }
    }
}

impl std::fmt::Display for AudioSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioSource::None => write!(f, "none"),
            AudioSource::System => write!(f, "system"),
            AudioSource::Mic => write!(f, "mic"),
            AudioSource::Both => write!(f, "both"),
        }
    }
}
