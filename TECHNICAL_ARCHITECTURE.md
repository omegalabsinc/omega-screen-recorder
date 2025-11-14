# Omega Screen Recorder - Technical Architecture & Development Guide

## Table of Contents
- [Architecture Overview](#architecture-overview)
- [System Components](#system-components)
- [Data Flow](#data-flow)
- [Technology Stack](#technology-stack)
- [Development Setup](#development-setup)
- [Component Details](#component-details)
- [Database Schema](#database-schema)
- [Performance Characteristics](#performance-characteristics)
- [Platform-Specific Implementation](#platform-specific-implementation)
- [Build & Deployment](#build--deployment)
- [Testing](#testing)
- [Troubleshooting Development Issues](#troubleshooting-development-issues)

## Architecture Overview

Omega Screen Recorder is built with a **channel-based concurrent pipeline architecture** using Rust's async/await with Tokio runtime. The system is designed for high-performance, low-latency screen capture with real-time encoding.

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Main Process (Tokio)                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌──────────────┐    ┌──────────────┐    ┌─────────────────┐  │
│  │   Screen     │───▶│   Bridge     │───▶│  Video Encoder  │  │
│  │   Capture    │    │   Thread     │    │   (Chunked)     │  │
│  │  (OS Thread) │    │ (Sync→Async) │    │                 │  │
│  └──────────────┘    └──────────────┘    └─────────────────┘  │
│         │                                          │             │
│         │ Frame Data                               │ Chunk       │
│         │ (std::mpsc)                             │ Metadata    │
│         ▼                                          ▼             │
│  ┌──────────────┐                        ┌─────────────────┐   │
│  │  Interaction │                        │  SQLite DB      │   │
│  │   Tracker    │                        │  (Async)        │   │
│  │ (OS Thread)  │                        └─────────────────┘   │
│  └──────────────┘                                 │             │
│         │                                          │             │
│         │ Events                                   │             │
│         ▼                                          ▼             │
│  ┌──────────────┐                        ┌─────────────────┐   │
│  │ JSONL Writer │                        │ Frame Metadata  │   │
│  │  (clicks)    │                        │   Tracking      │   │
│  └──────────────┘                        └─────────────────┘   │
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐                         │
│  │    Audio     │───▶│    Audio     │                         │
│  │   Capture    │    │  Processing  │                         │
│  │ (OS Thread)  │    │   (Async)    │                         │
│  └──────────────┘    └──────────────┘                         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Design Principles

1. **Separation of Concerns**: Each component has a single, well-defined responsibility
2. **Async-First**: Uses Tokio for all I/O operations (DB, file writes, channels)
3. **Zero-Copy Where Possible**: Minimizes data copying through channel-based communication
4. **Platform Abstraction**: Platform-specific code isolated in dedicated modules
5. **Graceful Degradation**: Components can fail independently without crashing the system
6. **Lock-Free Design**: Uses message passing over shared state

## System Components

### 1. CLI Parser (cli.rs)
**Purpose**: Command-line argument parsing and validation

**Technology**: `clap` with derive macros

**Key Structures**:
- `Cli`: Root command structure
- `Commands`: Enum of available commands (Screenshot, Record)
- `AudioSource`: Audio input configuration
- `RecordingType`: Task vs. Always-On mode

**Responsibilities**:
- Parse command-line arguments
- Validate parameter combinations
- Provide help/version information

### 2. Screen Capture (capture.rs)
**Purpose**: Cross-platform screen capture with multi-monitor support

**Technology**: `scrap` library for screen capture, `rdev` for cursor tracking

**Key Structures**:
- `ScreenCapture`: Main capture coordinator
- `Frame`: Captured frame data with metadata
- `MonitorSwitchDetector`: Tracks cursor across displays

**Responsibilities**:
- Initialize screen capturers for each display
- Capture frames at specified FPS
- Convert BGRA → RGB (remove alpha channel)
- Render cursor overlay on frames
- Detect and switch between displays (multi-monitor mode)
- Maintain consistent frame timing

**Threading Model**: Runs in OS thread (not Tokio) because `scrap::Capturer` is not Send

**Frame Format**: RGB24 (3 bytes per pixel, no alpha)

### 3. Display Info (display_info.rs)
**Purpose**: Multi-monitor detection and cursor-to-display mapping

**Technology**: `core-graphics` (macOS), `scrap` for cross-platform

**Key Functions**:
- `get_all_displays_with_bounds()`: Enumerate all displays with position/size
- `get_display_at_cursor()`: Determine which display contains cursor

**Responsibilities**:
- Get display physical bounds (x, y, width, height)
- Map cursor coordinates to display index
- Support multi-monitor layouts (horizontal, vertical, mixed)

### 4. Video Encoder (encoder.rs)
**Purpose**: Real-time H.264 encoding with chunking support

**Technology**: `ffmpeg-next` (FFmpeg 7.0 bindings)

**Key Structures**:
- `VideoEncoder`: H.264 encoder wrapper
- `FrameMetadata`: Per-frame encoding information
- `RecordingOutput`: Encoded video file reference

**Encoding Pipeline**:
1. Receive RGB24 frame from capture
2. Scale/pad if resolution mismatch (multi-monitor)
3. Convert RGB → YUV420P (optimized integer math)
4. Set PTS (Presentation Timestamp) = frame_count
5. Encode with H.264 (CRF-based quality)
6. Rescale timestamps (encoder time_base → stream time_base)
7. Write packet to MP4 container
8. Log metadata to database

**Quality Mapping**:
```rust
Quality 1-2  → CRF 35-39 (low quality)
Quality 3-5  → CRF 27-33 (medium quality)
Quality 6-8  → CRF 18-24 (high quality)
Quality 9-10 → CRF 12-15 (very high quality)
```

**Chunking Strategy**:
- Time-based: New chunk every N seconds (default: 10s)
- Each chunk is independent H.264 stream
- Keyframes at start of each chunk
- Metadata logged to database per chunk

**Performance Optimization**:
- Unsafe pointer arithmetic for YUV conversion (3x faster)
- Fixed-point integer math instead of floating-point
- Direct memory writes to FFmpeg frame buffers

### 5. Audio Capture (audio.rs)
**Purpose**: Cross-platform audio input capture

**Technology**: `cpal` (Cross-Platform Audio Library)

**Key Structures**:
- `AudioCapture`: Audio device manager
- `AudioSample`: Raw audio data

**Responsibilities**:
- Initialize audio input device (mic or system)
- Capture audio stream at device sample rate
- Convert stereo → mono if needed
- Send samples through channel

**Limitations**:
- Currently captures but doesn't encode to file
- System audio capture depends on OS support
- Audio and video are not synchronized (future improvement)

**Threading Model**: Runs in OS thread (cpal callback is not async)

### 6. Interaction Tracker (interactions.rs)
**Purpose**: Capture mouse and keyboard events with timestamps

**Technology**: `rdev` for global input hooks

**Key Structures**:
- `InteractionTracker`: Event capture coordinator
- `MouseEvent`: Click, move, scroll with timestamp
- `KeyboardEvent`: Key press/release with timestamp
- `ClickEvent`: JSONL format for task mode

**Modes**:
1. **Task Mode**: Always logs clicks to `clicks.jsonl`
2. **Always-On Mode**: Only logs if `--track-interactions` enabled

**Event Processing**:
- Mouse moves: Sampled (every 5th event) to reduce data
- Mouse clicks: Always captured with window context
- Keyboard: All press/release events captured
- Timestamps: Milliseconds from recording start

**Platform Features**:
- **macOS**: Active window info via `active-win-pos-rs` (requires Accessibility permission)
- **Other OS**: Window info not available

**Output Formats**:
- `clicks.jsonl`: One JSON object per line (task mode)
- `interactions.json`: Complete event history with metadata

### 7. Database (db.rs)
**Purpose**: Persistent storage for video chunks and frame metadata

**Technology**: `sqlx` with SQLite

**Key Structures**:
- `Database`: Async database connection pool
- `FrameInfo`: Frame record from database
- `VideoChunkInfo`: Chunk record from database

**Schema**: See [Database Schema](#database-schema) section

**Operations**:
- `insert_video_chunk()`: Create new chunk record
- `insert_frame()`: Insert frame with auto-incrementing offset
- `get_chunks_by_task_id()`: Retrieve all chunks for task
- `get_frames_by_task_id()`: Retrieve all frames for task

**Concurrency**: Connection pool (max 5 connections) for async operations

### 8. Error Handling (error.rs)
**Purpose**: Unified error type for all components

**Technology**: `thiserror` for derive macros

**Error Types**:
- `CaptureError`: Screen capture failures
- `AudioError`: Audio device failures
- `EncodingError`: Video encoding failures
- `ConfigError`: Configuration issues
- `InvalidParameter`: CLI argument validation

### 9. Screenshot (screenshot.rs)
**Purpose**: Single-frame capture and image export

**Technology**: `scrap` for capture, `image` for encoding

**Responsibilities**:
- Capture single frame from display
- Convert BGRA → RGB
- Encode to PNG/JPEG
- Write to file

**Supported Formats**: PNG, JPG, JPEG (detected from extension)

### 10. Main Coordinator (main.rs)
**Purpose**: Application entry point and orchestration

**Responsibilities**:
- Parse CLI arguments
- Initialize components (DB, capture, encoder, audio, tracker)
- Set up communication channels
- Spawn concurrent tasks
- Handle graceful shutdown (Ctrl+C)
- Coordinate task mode concatenation
- Export metadata on completion

**Channel Architecture**:
```rust
// Synchronous channel for capture thread
(frame_tx_std, frame_rx_std): std::sync::mpsc::channel()

// Async channel for encoder
(frame_tx, frame_rx): tokio::sync::mpsc::channel(60)

// Bridge thread: sync receiver → async sender
tokio::spawn(async move {
    while let Ok(frame) = frame_rx_std.recv() {
        frame_tx.send(frame).await
    }
})

// Audio channel (async)
(audio_tx, audio_rx): tokio::sync::mpsc::channel(1000)
```

## Data Flow

### Recording Session Lifecycle

```
1. CLI Parsing
   └─▶ Validate parameters
       └─▶ Create output directories

2. Component Initialization
   ├─▶ Database: Connect to SQLite
   ├─▶ Screen Capture: Initialize capturers
   ├─▶ Audio: Initialize device
   └─▶ Interaction Tracker: Start event listener

3. Channel Setup
   ├─▶ Frame: sync channel → bridge → async channel
   └─▶ Audio: async channel

4. Spawn Concurrent Tasks
   ├─▶ Capture Thread (OS): frame_tx_std.send(frame)
   ├─▶ Bridge Task (Tokio): sync→async relay
   ├─▶ Encoder Task (Tokio): frame_rx.recv() → encode → DB
   ├─▶ Audio Thread (OS): audio_tx.send(sample)
   ├─▶ Audio Task (Tokio): audio_rx.recv() → process
   └─▶ Tracker Thread (OS): log clicks/keyboard

5. Recording Loop
   ├─▶ Capture: 30 FPS timer → capture frame → send
   ├─▶ Encoder: receive → encode → write chunk → log metadata
   ├─▶ Audio: device callback → send samples
   └─▶ Tracker: global hooks → log events

6. Shutdown (Ctrl+C or duration reached)
   ├─▶ Set running flag to false
   ├─▶ Wait for capture thread to finish
   ├─▶ Wait for bridge task to drain
   ├─▶ Wait for encoder task to flush
   ├─▶ Wait for audio task to complete
   └─▶ Save interaction data to JSON

7. Task Mode Finalization (if --is-final)
   ├─▶ Query all chunks from database
   ├─▶ Detect resolution changes
   ├─▶ Create FFmpeg concat list
   ├─▶ Run FFmpeg with normalization (if needed)
   ├─▶ Export frame metadata to JSON
   └─▶ Clean up temporary files
```

### Frame Processing Pipeline

```
┌─────────────────────────────────────────────────────────────────┐
│ 1. Capture (OS Thread)                                           │
│    - Wait for next frame (blocking)                             │
│    - Convert BGRA → RGB (remove alpha)                          │
│    - Draw cursor overlay                                        │
│    - Create Frame struct with metadata                          │
│    - Send via sync channel                                      │
└────────────────────┬────────────────────────────────────────────┘
                     │ std::sync::mpsc
                     ▼
┌─────────────────────────────────────────────────────────────────┐
│ 2. Bridge (Tokio Task)                                           │
│    - Receive from sync channel (blocking in task)               │
│    - Forward to async channel                                   │
└────────────────────┬────────────────────────────────────────────┘
                     │ tokio::sync::mpsc
                     ▼
┌─────────────────────────────────────────────────────────────────┐
│ 3. Encoder (Tokio Task)                                          │
│    - Check if new chunk needed (time-based)                     │
│    - Scale/pad frame if resolution mismatch                     │
│    - Convert RGB → YUV420P                                       │
│    - Set PTS = frame_count                                      │
│    - Encode with H.264                                          │
│    - Rescale timestamps                                         │
│    - Write packet to MP4                                        │
└────────────────────┬────────────────────────────────────────────┘
                     │ Async DB insert
                     ▼
┌─────────────────────────────────────────────────────────────────┐
│ 4. Database (SQLite)                                             │
│    - Insert frame metadata                                      │
│    - Track keyframes, PTS, display info                        │
│    - Link to video chunk                                        │
└─────────────────────────────────────────────────────────────────┘
```

### Multi-Monitor Switching Logic

```
┌─────────────────────────────────────────────────────────────────┐
│ Monitor Switch Detection (1 second interval)                     │
└─────────────────────────────────────────────────────────────────┘
                     │
                     ▼
        ┌────────────────────────┐
        │ Get current cursor (x,y)│
        └────────────┬───────────┘
                     │
                     ▼
        ┌────────────────────────┐
        │ Map to display index   │
        └────────────┬───────────┘
                     │
                     ▼
        ┌────────────────────────┐
        │ Same as current?       │
        └────────┬───────┬───────┘
                YES     NO
                 │       │
                 │       ▼
                 │  ┌─────────────────────┐
                 │  │ Same as pending?    │
                 │  └────┬──────────┬─────┘
                 │      YES        NO
                 │       │          │
                 │       ▼          ▼
                 │  ┌─────────┐  ┌─────────────┐
                 │  │ Count++ │  │ New pending │
                 │  └────┬────┘  └──────┬──────┘
                 │       │              │
                 │       ▼              │
                 │  ┌─────────┐        │
                 │  │ Count≥2?│        │
                 │  └────┬────┘        │
                 │      YES             │
                 │       │              │
                 │       ▼              │
                 │  ┌─────────────┐    │
                 │  │ SWITCH!     │    │
                 │  │ - Stop old  │    │
                 │  │ - Start new │    │
                 │  │ - Update W/H│    │
                 │  └─────────────┘    │
                 │                     │
                 ▼◀────────────────────┘
        ┌────────────────────────┐
        │ Continue capturing     │
        └────────────────────────┘
```

**Debouncing**: Requires 2+ consecutive checks on new display before switching (prevents flickering)

## Technology Stack

### Core Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1.35 | Async runtime, channels, tasks |
| `clap` | 4.4 | CLI argument parsing |
| `scrap` | 0.5 | Cross-platform screen capture |
| `ffmpeg-next` | 7.0 | Video encoding (H.264, MP4) |
| `cpal` | 0.15 | Cross-platform audio capture |
| `rdev` | 0.5 | Global mouse/keyboard hooks |
| `sqlx` | 0.7 | Async SQLite with compile-time SQL checks |
| `serde` / `serde_json` | 1.0 | JSON serialization |
| `chrono` | 0.4 | Timestamp handling |
| `image` | 0.24 | Screenshot encoding |
| `anyhow` / `thiserror` | 1.0 | Error handling |

### Platform-Specific Dependencies

| Platform | Crate | Purpose |
|----------|-------|---------|
| macOS | `core-graphics` | Display bounds and geometry |
| macOS | `active-win-pos-rs` | Active window detection |
| Windows | `windows` | Win32 API for display info |

### System Requirements

#### Required
- **FFmpeg 7.0** libraries (libavcodec, libavformat, libavutil, libswscale)
- **Rust 1.70+** (tested on 1.80)
- **pkg-config** (for FFmpeg discovery)

#### Optional
- **SQLite 3** (bundled via sqlx if not found)

#### Platform-Specific
- **macOS**:
  - Xcode Command Line Tools
  - Screen Recording permission (System Settings)
  - Accessibility permission (for window info)

- **Linux**:
  - X11 or Wayland
  - Development headers for FFmpeg
  - ALSA or PulseAudio

- **Windows**:
  - Visual Studio Build Tools
  - FFmpeg DLLs in PATH or specified via `--ffmpeg-path`

## Development Setup

### Initial Setup

```bash
# Clone repository
git clone https://github.com/omegalabsinc/omega-screen-recorder.git
cd omega-screen-recorder

# Install FFmpeg v7
# macOS
brew install ffmpeg pkg-config

# Linux (Ubuntu/Debian)
sudo apt-get update
sudo apt-get install \
  libavformat-dev \
  libavcodec-dev \
  libavutil-dev \
  libswscale-dev \
  pkg-config \
  libasound2-dev

# Linux (Fedora)
sudo dnf install \
  ffmpeg-devel \
  pkg-config \
  alsa-lib-devel

# Build debug version
cargo build

# Build release version (optimized)
cargo build --release

# Run tests
cargo test

# Check for errors without building
cargo check
```

### Development Workflow

```bash
# Run with debug logging
RUST_LOG=debug ./target/debug/screenrec --verbose record --duration 10

# Run with specific module logging
RUST_LOG=screenrec::encoder=debug,screenrec::capture=debug ./target/debug/screenrec record --duration 10

# Check database after recording
sqlite3 ~/.omega/db.sqlite
> SELECT * FROM video_chunks ORDER BY id DESC LIMIT 5;
> SELECT COUNT(*) FROM frames;
> .quit

# Format code
cargo fmt

# Run linter
cargo clippy

# Build documentation
cargo doc --open

# Watch for changes and rebuild (requires cargo-watch)
cargo install cargo-watch
cargo watch -x 'build --release'
```

### Debugging Tips

#### Enable Verbose Logging
```rust
// In main.rs, set log level
env_logger::Builder::from_env(
    env_logger::Env::default().default_filter_or("debug")
).init();
```

#### Print Frame Information
```rust
// In encoder.rs, after encoding
log::info!("Frame {}: PTS={:?}, keyframe={}, size={}x{}",
    self.frame_count,
    metadata.pts,
    metadata.is_keyframe,
    width,
    height
);
```

#### Inspect Channels
```rust
// Check channel capacity
log::debug!("Frame channel capacity: {}", frame_rx.capacity());
log::debug!("Frame channel len: {}", frame_rx.len());
```

#### Database Queries
```sql
-- Check frame distribution across chunks
SELECT video_chunk_id, COUNT(*) as frame_count
FROM frames
GROUP BY video_chunk_id;

-- Find keyframes
SELECT * FROM frames WHERE is_keyframe = 1 LIMIT 10;

-- Check display switching
SELECT display_index, display_width, display_height, COUNT(*)
FROM frames
GROUP BY display_index, display_width, display_height;
```

## Component Details

### Screen Capture Implementation

#### Single Monitor Mode (Optimized Path)
- Zero overhead for most users
- One `scrap::Capturer` instance
- No display detection logic
- Minimal latency

#### Multi-Monitor Mode (2+ Displays)
- Creates capturer for each display
- HashMap of capturers: `HashMap<usize, Capturer>`
- MonitorSwitchDetector checks cursor every N seconds
- Switches active capturer based on cursor position
- Frame dimensions can change mid-recording

**Capture Loop**:
```rust
loop {
    // 1. Check for stop conditions
    if should_stop() { break; }

    // 2. Check for monitor switch (multi-monitor only)
    if let Some(new_display) = switch_detector.check_for_switch() {
        current_capturer = capturers.get_mut(&new_display)?;
        width = current_capturer.width();
        height = current_capturer.height();
    }

    // 3. Capture frame
    match current_capturer.frame() {
        Ok(frame) => {
            // 4. Convert BGRA → RGB
            let rgb = convert_bgra_to_rgb(frame);

            // 5. Draw cursor
            draw_cursor(&mut rgb, cursor_x, cursor_y);

            // 6. Send frame
            tx.send(Frame { data: rgb, ... })?;
        }
        Err(WouldBlock) => continue,
        Err(e) => return Err(e),
    }

    // 7. Sleep to maintain frame rate
    sleep(frame_duration - elapsed);
}
```

### Video Encoder Implementation

#### H.264 Configuration
```rust
// Encoder settings
video_encoder.set_width(width as u32);
video_encoder.set_height(height as u32);
video_encoder.set_format(Pixel::YUV420P);
video_encoder.set_time_base(Rational::new(1, fps as i32));
video_encoder.set_frame_rate(Some(Rational::new(fps as i32, 1)));

// CRF-based quality
let crf = quality_to_crf(quality); // 1-10 → 12-39
opts.set("crf", &crf.to_string());
opts.set("preset", "medium"); // medium = good speed/quality
```

#### Timestamp Management
```rust
// Encoder time_base: 1/fps (e.g., 1/30)
// Frame PTS in encoder: 0, 1, 2, 3, ...
yuv_frame.set_pts(Some(frame_count as i64));

// Stream time_base: 1/90000 (MP4 standard)
stream.set_time_base(Rational(1, 90000));

// Rescale packet timestamps
packet.rescale_ts(
    Rational(1, fps as i32),  // from encoder
    Rational(1, 90000)         // to stream
);
```

#### YUV Conversion Optimization
```rust
// Optimized integer math (no floating-point)
unsafe {
    // Y = 0.299*R + 0.587*G + 0.114*B
    // Using fixed-point: (77*R + 150*G + 29*B) >> 8
    let y_val = ((77 * r + 150 * g + 29 * b) >> 8) as u8;

    // U = -0.169*R - 0.331*G + 0.500*B + 128
    let u_val = (((-43 * r - 85 * g + 128 * b) >> 8) + 128).clamp(0, 255);

    // V = 0.500*R - 0.419*G - 0.081*B + 128
    let v_val = (((128 * r - 107 * g - 21 * b) >> 8) + 128).clamp(0, 255);
}
```

#### Chunk Management
```rust
// Time-based chunking
if frames_in_current_chunk >= frames_per_chunk {
    // 1. Finish current encoder
    current_encoder.finish()?;
    chunk_outputs.push(output);

    // 2. Create new encoder for next chunk
    chunk_index += 1;
    let chunk_path = base_dir.join(format!("{}.mp4", timestamp));
    current_encoder = VideoEncoder::new(&chunk_path, ...)?;

    // 3. Insert new chunk into database
    db.insert_video_chunk(...).await?;

    // 4. Reset frame counter
    frames_in_current_chunk = 0;
}
```

#### Multi-Resolution Normalization (Task Mode)
```rust
// Detect different resolutions in frames
let mut resolutions = HashSet::new();
for frame in frames {
    resolutions.insert((frame.display_width, frame.display_height));
}

if resolutions.len() > 1 {
    // Find maximum dimensions
    let (max_w, max_h) = resolutions.iter()
        .fold((0, 0), |(max_w, max_h), &(w, h)| {
            (max_w.max(w), max_h.max(h))
        });

    // FFmpeg filter: scale to fit + pad with black bars
    let filter = format!(
        "scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2:black",
        max_w, max_h, max_w, max_h
    );

    ffmpeg -f concat -safe 0 -i concat_list.txt \
           -vf "$filter" \
           -c:v libx264 -preset medium -crf 23 \
           final.mp4
}
```

### Interaction Tracking Implementation

#### Event Listener
```rust
let handle = std::thread::spawn(move || {
    let mut last_mouse_x = 0.0;
    let mut last_mouse_y = 0.0;

    let callback = move |event: Event| {
        match event.event_type {
            MouseMove { x, y } => {
                last_mouse_x = x;
                last_mouse_y = y;

                // Always update for cursor rendering
                update_cursor_position(x as i32, y as i32);

                // Optionally log movement (sampled)
                if track_movements && counter % sample_rate == 0 {
                    log_mouse_event(x, y, "move");
                }
            }

            ButtonPress(button) => {
                // Use last known position
                log_click(last_mouse_x, last_mouse_y, button);

                // Task mode: write to JSONL
                if let Some(ref task_id) = task_id {
                    let (app, title) = get_active_window_info();
                    write_click_jsonl(x, y, button, task_id, app, title)?;
                }
            }

            KeyPress(key) => {
                log_keyboard_event(key, "press");
            }

            _ => {}
        }
    };

    // Blocking event loop
    rdev::listen(callback)?;
});
```

#### Active Window Detection (macOS)
```rust
#[cfg(target_os = "macos")]
fn get_active_window_info() -> (String, String) {
    match active_win_pos_rs::get_active_window() {
        Ok(window) => (window.app_name, window.title),
        Err(_) => {
            // Requires Accessibility permission
            log::warn!("Cannot get window info - grant Accessibility permission");
            ("Unknown".to_string(), "Unknown".to_string())
        }
    }
}
```

### Database Operations

#### Frame Insertion with Auto-Increment
```rust
pub async fn insert_frame(&self, device_name: &str, ...) -> Result<i64> {
    let mut tx = self.pool.begin().await?;

    // 1. Get latest video chunk
    let chunk_id: i64 = sqlx::query_scalar(
        "SELECT id FROM video_chunks
         WHERE device_name = ?1
         ORDER BY id DESC LIMIT 1"
    )
    .bind(device_name)
    .fetch_one(&mut *tx)
    .await?;

    // 2. Calculate next offset_index
    let offset: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(offset_index), -1) + 1
         FROM frames
         WHERE video_chunk_id = ?1"
    )
    .bind(chunk_id)
    .fetch_one(&mut *tx)
    .await?;

    // 3. Insert frame
    sqlx::query(
        "INSERT INTO frames (video_chunk_id, offset_index, ...)
         VALUES (?1, ?2, ...)"
    )
    .bind(chunk_id)
    .bind(offset)
    ...
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(result.last_insert_rowid())
}
```

#### Task Concatenation Query
```rust
// Get all chunks for a task, ordered by creation time
let chunks = sqlx::query_as::<_, VideoChunkInfo>(
    "SELECT * FROM video_chunks
     WHERE task_id = ?1
     ORDER BY created_at ASC"
)
.bind(task_id)
.fetch_all(&pool)
.await?;

// Get all frames with display info
let frames = sqlx::query_as::<_, FrameInfo>(
    "SELECT f.*, vc.file_path
     FROM frames f
     JOIN video_chunks vc ON f.video_chunk_id = vc.id
     WHERE vc.task_id = ?1
     ORDER BY vc.created_at ASC, f.offset_index ASC"
)
.bind(task_id)
.fetch_all(&pool)
.await?;
```

## Database Schema

### video_chunks Table
```sql
CREATE TABLE video_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL,              -- Path to .mp4 file
    device_name TEXT NOT NULL,            -- Hostname of recording device
    recording_type TEXT,                  -- "task" or "always_on"
    task_id TEXT,                         -- Task identifier (for task mode)
    chunk_index INTEGER,                  -- Sequential chunk number
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

### frames Table
```sql
CREATE TABLE frames (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    video_chunk_id INTEGER NOT NULL,      -- Foreign key to video_chunks
    offset_index INTEGER NOT NULL,        -- Frame index within chunk (0, 1, 2, ...)
    timestamp TIMESTAMP NOT NULL,         -- Capture timestamp (UTC)
    device_name TEXT,                     -- Hostname
    is_keyframe INTEGER DEFAULT 0,        -- 1 if keyframe, 0 otherwise
    pts INTEGER,                          -- Presentation timestamp (encoder time_base)
    dts INTEGER,                          -- Decode timestamp (encoder time_base)
    display_index INTEGER DEFAULT 0,      -- Which monitor (0 = primary)
    display_width INTEGER,                -- Resolution at capture time
    display_height INTEGER,               -- Resolution at capture time
    FOREIGN KEY (video_chunk_id) REFERENCES video_chunks(id)
);

CREATE INDEX idx_frames_video_chunk_id ON frames(video_chunk_id);
CREATE INDEX idx_frames_keyframe ON frames(is_keyframe) WHERE is_keyframe = 1;
```

### Example Data

**video_chunks**:
```
id | file_path                                    | device_name | recording_type | task_id      | chunk_index | created_at
1  | ~/.omega/data/tasks/demo/2025-01-14_10-30... | MacBook-Pro | task           | demo         | 0           | 2025-01-14 10:30:00
2  | ~/.omega/data/tasks/demo/2025-01-14_10-30... | MacBook-Pro | task           | demo         | 1           | 2025-01-14 10:30:10
3  | ~/.omega/data/always_on/2025-01-14_11-00-... | MacBook-Pro | always_on      | NULL         | 0           | 2025-01-14 11:00:00
```

**frames**:
```
id  | video_chunk_id | offset_index | timestamp           | is_keyframe | pts | dts | display_index | display_width | display_height
1   | 1              | 0            | 2025-01-14 10:30:00 | 1           | 0   | 0   | 0             | 1920          | 1080
2   | 1              | 1            | 2025-01-14 10:30:00 | 0           | 1   | 1   | 0             | 1920          | 1080
301 | 2              | 0            | 2025-01-14 10:30:10 | 1           | 0   | 0   | 1             | 2560          | 1440
302 | 2              | 1            | 2025-01-14 10:30:10 | 0           | 1   | 1   | 1             | 2560          | 1440
```

## Performance Characteristics

### Benchmarks (M1 MacBook Pro, 1080p @ 30fps)

| Metric | Value | Notes |
|--------|-------|-------|
| CPU Usage | 15-25% | Single core, H.264 CRF=23 |
| Memory | ~50MB | Excluding video file size |
| Frame Latency | <10ms | Capture to encode start |
| Disk I/O | ~5-10 MB/s | Depends on quality setting |
| Database Writes | ~30/sec | Frame inserts (batched) |

### CPU Profile (30fps recording)

```
Component             | CPU % | Notes
----------------------|-------|----------------------------------
Screen Capture        | 8-12% | scrap library overhead
RGB→YUV Conversion    | 3-5%  | Optimized unsafe code
H.264 Encoding        | 5-10% | FFmpeg libx264 (CRF=23)
Cursor Rendering      | 1-2%  | Pixel drawing
Database Writes       | 0.5%  | Async SQLite inserts
Interaction Tracking  | 0.5%  | rdev event processing
Other (async runtime) | 1-2%  | Tokio overhead
```

### Memory Usage

```
Component             | Memory | Notes
----------------------|--------|----------------------------------
Frame Buffer (RGB)    | 6MB    | 1920x1080x3 bytes
YUV Frame Buffer      | 3MB    | YUV420P format
Channel Queues        | 2-5MB  | Frame/audio sample buffers
FFmpeg Internal       | 5-10MB | Encoder state
Database Connection   | 1-2MB  | SQLite pool
Interaction Events    | 1-5MB  | Grows with recording duration
Other (Rust runtime)  | 10MB   | Stack, heap, etc.
```

### Optimization Strategies

1. **YUV Conversion**: Unsafe pointer arithmetic (3x faster than safe)
2. **Channel Buffering**: Tuned buffer sizes (60 frames, 1000 audio samples)
3. **Database Batching**: Transaction per frame (could be improved with batching)
4. **Cursor Rendering**: Pre-computed pixel art (no image loading)
5. **Frame Rate**: Sleep-based timing (simple, effective)
6. **Multi-Monitor**: Only enabled when 2+ displays detected

### Bottlenecks & Future Improvements

1. **Screen Capture**: `scrap` library overhead (10-15% CPU)
   - **Solution**: Direct platform APIs (CGDisplayStream on macOS)

2. **Audio Not Encoded**: Currently just captured, not saved
   - **Solution**: AAC encoding with FFmpeg, sync with video

3. **Database Writes**: One transaction per frame
   - **Solution**: Batch inserts (e.g., 30 frames per transaction)

4. **No GPU Acceleration**: All encoding on CPU
   - **Solution**: Hardware encoding (VideoToolbox on macOS, NVENC on NVIDIA)

5. **Single-Threaded Encoding**: H.264 encoder runs in one thread
   - **Solution**: FFmpeg threading options, or parallel chunking

## Platform-Specific Implementation

### macOS

**Screen Capture**: `scrap` uses `CGDisplayStream` API
**Display Info**: `core-graphics` for bounds and geometry
**Window Detection**: `active-win-pos-rs` (requires Accessibility permission)
**Audio**: CoreAudio via `cpal`

**Permissions Required**:
```
- Screen Recording: System Settings → Privacy & Security → Screen Recording
- Accessibility: System Settings → Privacy & Security → Accessibility
```

**FFmpeg Installation**:
```bash
brew install ffmpeg pkg-config
```

### Linux

**Screen Capture**: `scrap` uses X11 or Wayland
**Display Info**: X11 RandR extension
**Window Detection**: Not implemented
**Audio**: ALSA or PulseAudio via `cpal`

**Dependencies**:
```bash
# Ubuntu/Debian
sudo apt-get install \
  libavformat-dev libavcodec-dev libavutil-dev libswscale-dev \
  libxcb1-dev libxrandr-dev \
  libasound2-dev \
  pkg-config

# Fedora
sudo dnf install \
  ffmpeg-devel \
  libxcb-devel libXrandr-devel \
  alsa-lib-devel \
  pkg-config
```

### Windows

**Screen Capture**: `scrap` uses DXGI (DirectX Graphics Infrastructure)
**Display Info**: Win32 API (`EnumDisplayMonitors`)
**Window Detection**: Not implemented
**Audio**: WASAPI via `cpal`

**Dependencies**:
- Visual Studio Build Tools (for Rust compilation)
- FFmpeg DLLs (download from ffmpeg.org)

**FFmpeg Setup**:
1. Download FFmpeg shared build from https://ffmpeg.org/download.html
2. Extract DLLs to PATH or use `--ffmpeg-path` flag
3. Set `FFMPEG_DIR` environment variable (optional)

## Build & Deployment

### Release Build

```bash
# Standard release build
cargo build --release

# Optimized release build (specified in Cargo.toml)
# - opt-level = 3
# - lto = true (link-time optimization)
# - codegen-units = 1 (slower build, faster runtime)
# - strip = true (remove debug symbols)
cargo build --release

# Cross-compilation example (macOS → Linux)
cargo install cross
cross build --release --target x86_64-unknown-linux-gnu
```

### Binary Size Optimization

Current size: ~15-20 MB (release, stripped)

**Further optimization**:
```toml
[profile.release]
opt-level = "z"        # Optimize for size
lto = true
codegen-units = 1
strip = true
panic = "abort"        # Remove panic unwinding code
```

### Static Linking (Linux)

```bash
# Install musl target
rustup target add x86_64-unknown-linux-musl

# Build with musl (static binary)
cargo build --release --target x86_64-unknown-linux-musl

# Note: FFmpeg must also be statically linked (complex)
```

### Packaging

**macOS (.app bundle)**:
```bash
# Create app structure
mkdir -p ScreenRec.app/Contents/MacOS
mkdir -p ScreenRec.app/Contents/Resources

# Copy binary
cp target/release/screenrec ScreenRec.app/Contents/MacOS/

# Create Info.plist with permissions
# (NSScreenCaptureUsageDescription, NSAccessibilityUsageDescription)
```

**Windows (installer)**:
- Use WiX Toolset or Inno Setup
- Include FFmpeg DLLs
- Create Start Menu shortcuts

**Linux (package)**:
```bash
# Debian package
cargo install cargo-deb
cargo deb

# RPM package
cargo install cargo-rpm
cargo rpm build
```

### Docker (Linux only)

```dockerfile
FROM rust:1.80 AS builder

RUN apt-get update && apt-get install -y \
    libavformat-dev libavcodec-dev libavutil-dev libswscale-dev \
    pkg-config

WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/screenrec /usr/local/bin/
ENTRYPOINT ["screenrec"]
```

## Testing

### Unit Tests

```bash
# Run all tests
cargo test

# Run specific module tests
cargo test encoder::tests
cargo test db::tests

# Run with output
cargo test -- --nocapture

# Run with specific filter
cargo test test_frame_insertion
```

### Integration Tests

```bash
# Test full recording pipeline
cargo test --test integration_tests

# Test with actual screen capture (requires display)
cargo test --test recording_tests -- --ignored
```

### Example Test
```rust
#[tokio::test]
async fn test_video_chunk_insertion() {
    let db = Database::new(":memory:").await.unwrap();

    let chunk_id = db.insert_video_chunk(
        "/tmp/test.mp4",
        "test-device",
        Some("task"),
        Some("test-task"),
        Some(0),
    ).await.unwrap();

    assert!(chunk_id > 0);

    let chunks = db.get_chunks_by_task_id("test-task").await.unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].file_path, "/tmp/test.mp4");
}
```

### Manual Testing Checklist

- [ ] Screenshot capture (PNG, JPEG)
- [ ] Video recording (30fps, 60fps)
- [ ] Audio capture (system, mic, both, none)
- [ ] Multi-monitor switching
- [ ] Interaction tracking (clicks, keyboard)
- [ ] Task mode workflow (multiple sessions, --is-final)
- [ ] Graceful shutdown (Ctrl+C)
- [ ] Duration limit
- [ ] Custom output paths
- [ ] Quality settings (1-10)
- [ ] Chunk duration
- [ ] Database integrity
- [ ] Multi-resolution normalization
- [ ] Click JSONL format
- [ ] Frame metadata export

### Performance Testing

```bash
# CPU usage monitoring
screenrec record --duration 60 &
PID=$!
sleep 5
ps -p $PID -o %cpu,rss,vsz

# Memory leak check (requires valgrind on Linux)
valgrind --leak-check=full ./target/release/screenrec record --duration 30

# Frame timing analysis
screenrec --verbose record --duration 10 2>&1 | grep "Frame"

# Database growth
ls -lh ~/.omega/db.sqlite
sqlite3 ~/.omega/db.sqlite "SELECT COUNT(*) FROM frames"
```

## Troubleshooting Development Issues

### FFmpeg Not Found

**Error**: `error: linking with 'cc' failed` or `cannot find -lavcodec`

**Solutions**:
```bash
# macOS
brew install ffmpeg pkg-config
export PKG_CONFIG_PATH="/usr/local/opt/ffmpeg/lib/pkgconfig"

# Linux
sudo apt-get install libavformat-dev libavcodec-dev
export PKG_CONFIG_PATH="/usr/lib/pkgconfig"

# Manual path
export FFMPEG_DIR="/path/to/ffmpeg"
```

### scrap Build Errors

**Error**: Platform-specific dependencies missing

**Solutions**:
```bash
# macOS: Xcode Command Line Tools
xcode-select --install

# Linux: X11 development headers
sudo apt-get install libxcb1-dev libxrandr-dev

# Windows: Visual Studio Build Tools
# Download from: https://visualstudio.microsoft.com/downloads/
```

### SQLite Errors

**Error**: `database is locked`

**Solutions**:
- Increase connection pool size in `db.rs`
- Use WAL mode: `PRAGMA journal_mode=WAL`
- Ensure single writer (shouldn't happen with current design)

### Runtime Permissions (macOS)

**Error**: `Failed to capture frame` or blank screen

**Solutions**:
1. Grant Screen Recording permission:
   - System Settings → Privacy & Security → Screen Recording
   - Add Terminal or your terminal app
   - Restart terminal

2. Grant Accessibility permission (for window info):
   - System Settings → Privacy & Security → Accessibility
   - Add Terminal or your terminal app

### Audio Capture Issues

**Error**: `No input device found`

**Solutions**:
```bash
# macOS: Check audio devices
system_profiler SPAudioDataType

# Linux: Check ALSA devices
arecord -l

# Grant microphone permission (macOS)
# System Settings → Privacy & Security → Microphone
```

### High CPU Usage

**Causes**:
- FPS too high (try 30 or lower)
- Quality too high (try 6-8 instead of 10)
- Multi-monitor overhead
- Debug build instead of release

**Solutions**:
```bash
# Use release build
cargo build --release

# Lower settings
screenrec record --fps 30 --quality 7

# Profile with perf (Linux)
perf record -g ./target/release/screenrec record --duration 10
perf report
```

### Frame Drops

**Symptoms**: Jerky video, missing frames

**Causes**:
- System too slow for requested FPS
- Disk I/O bottleneck
- Channel buffer full

**Solutions**:
- Lower FPS
- Use SSD for output
- Increase channel buffer size in code
- Close other applications

### Database Connection Pool Exhausted

**Error**: `timed out waiting for connection`

**Solution**:
```rust
// In db.rs, increase max_connections
SqlitePoolOptions::new()
    .max_connections(10)  // Increase from 5
    .connect(&db_url)
    .await?
```

## Future Enhancements

### Roadmap
1. **Audio Encoding**: AAC encoding with A/V sync
2. **GPU Acceleration**: Hardware encoding (VideoToolbox, NVENC, VAAPI)
3. **Live Streaming**: RTMP output
4. **Region Selection**: Capture specific window or area
5. **Annotation**: Real-time drawing during recording
6. **Webcam Overlay**: Picture-in-picture support
7. **API/Library Mode**: Embed recorder in other applications
8. **Cloud Storage**: Auto-upload to S3, GCS, etc.
9. **Format Support**: WebM, animated GIF
10. **Advanced Editing**: Cut, trim, merge via CLI

### Contributing

See CONTRIBUTING.md for:
- Code style guidelines
- Pull request process
- Testing requirements
- Documentation standards

### License

MIT License - see LICENSE file

---

**Omega Labs** | [Website](https://omega.inc) | [GitHub](https://github.com/omegalabsinc)
