# Omega Screen Recorder - Features & Usage Guide

## Table of Contents
- [Overview](#overview)
- [Features](#features)
- [Installation](#installation)
- [Commands](#commands)
  - [Screenshot](#screenshot)
  - [Record](#record)
- [Recording Modes](#recording-modes)
- [Command Line Flags Reference](#command-line-flags-reference)
- [Output Files](#output-files)
- [Usage Examples](#usage-examples)
- [Tips & Best Practices](#tips--best-practices)

## Overview

Omega Screen Recorder (`screenrec`) is a high-performance, cross-platform CLI tool for capturing screenshots and recording screen video with advanced features like:
- Direct MP4 encoding with H.264 codec
- Multi-monitor support with automatic display switching
- Interaction tracking (mouse clicks, keyboard events)
- Task-based recording with chunk management
- Audio capture (system audio, microphone, or both)
- Cursor rendering in recordings

## Features

### Core Capabilities
- **Screenshot Capture**: Capture high-quality screenshots in PNG or JPEG format
- **Video Recording**: Record screen at configurable FPS (1-60) with MP4 output
- **Audio Support**: Capture system audio, microphone, or both simultaneously
- **Multi-Monitor**: Automatic detection and switching between displays based on cursor position
- **Cursor Tracking**: Renders cursor position in the recorded video
- **Interaction Tracking**: Records mouse clicks, movements, and keyboard events
- **Chunked Recording**: Time-based video chunking for long recordings
- **Task Mode**: Special mode for task-based workflows with automatic concatenation

### Performance Features
- **Direct MP4 Encoding**: No intermediate files, direct H.264 encoding
- **Low CPU Usage**: Optimized for <30% CPU usage during recording
- **Efficient Memory**: Minimal memory footprint with channel-based pipeline
- **Adjustable Quality**: 10-level quality scale (1-10)
- **Frame Rate Control**: Configurable FPS from 1 to 60

### Storage & Metadata
- **SQLite Database**: Stores frame metadata, chunk information, and timestamps
- **Frame Tracking**: Every frame is logged with PTS, DTS, keyframe status
- **Click Logging**: JSONL format for click events (in task mode)
- **Interaction JSON**: Complete mouse/keyboard event history
- **Display Metadata**: Tracks display index and resolution changes

## Installation

### From Source (Requires FFmpeg v7)

```bash
# Clone the repository
git clone https://github.com/omegalabsinc/omega-screen-recorder.git
cd omega-screen-recorder

# Install FFmpeg v7 (required dependency)
# macOS
brew install ffmpeg pkg-config

# Linux
sudo apt-get install libavformat-dev libavcodec-dev libavutil-dev libswscale-dev pkg-config

# Windows
# Download FFmpeg shared libraries from https://ffmpeg.org/download.html

# Build the project
cargo build --release

# The binary will be at: ./target/release/screenrec
```

### Via npm (Windows x64 / macOS with custom binary)

```bash
npm install -g @omega/screenrec-cli
screenrec --help
```

## Commands

### Screenshot

Capture a screenshot of your display.

```bash
screenrec screenshot [OPTIONS]
```

**Options:**
- `-o, --output <PATH>` - Output file path (default: `screenshot.png`)
- `-d, --display <NUM>` - Display to capture (default: `0` for primary)
- `-v, --verbose` - Enable verbose logging

**Examples:**
```bash
# Take a screenshot (default output)
screenrec screenshot

# Save to specific location
screenrec screenshot --output ~/Desktop/my-screenshot.png

# Capture secondary display
screenrec screenshot --display 1 --output monitor2.jpg
```

### Record

Record screen video with audio and interaction tracking.

```bash
screenrec record [OPTIONS]
```

## Recording Modes

### 1. Always-On Mode (Default)
Continuous recording mode for general-purpose screen capture.

```bash
screenrec record --duration 60
```

**Characteristics:**
- Saves to `~/.omega/data/always_on/` by default
- Creates timestamped video chunks
- Interaction tracking is optional (use `--track-interactions`)
- No automatic concatenation

### 2. Task Mode
Special mode for task-based workflows with automatic video concatenation.

```bash
screenrec record --recording-type task --task-id my-task-123
```

**Characteristics:**
- Saves to `~/.omega/data/tasks/<task-id>/`
- **Always tracks clicks** to `clicks.jsonl` (regardless of flags)
- Creates video chunks with metadata
- Use `--is-final` to concatenate all chunks into `final.mp4`
- Exports frame metadata to JSON
- Handles multi-resolution videos with normalization

**Task Mode Workflow:**
```bash
# 1. Start first recording session
screenrec record --recording-type task --task-id project-demo --duration 60

# 2. Start additional recording sessions (same task)
screenrec record --recording-type task --task-id project-demo --duration 60

# 3. Final recording session - concatenate all chunks
screenrec record --recording-type task --task-id project-demo --duration 60 --is-final

# Output:
# - ~/.omega/data/tasks/project-demo/final.mp4 (concatenated video)
# - ~/.omega/data/tasks/project-demo/project-demo_frames.json (metadata)
# - ~/.omega/data/tasks/project-demo/clicks.jsonl (click events)
```

## Command Line Flags Reference

### Global Flags
- `-v, --verbose` - Enable debug logging

### Recording Flags

#### Output & Duration
| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `-o, --output` | PATH | `~/.omega/data/` | Custom output directory |
| `-d, --duration` | SECONDS | `0` | Recording duration (0 = unlimited, Ctrl+C to stop) |

#### Video Quality
| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `-f, --fps` | NUMBER | `30` | Frames per second (1-60) |
| `-q, --quality` | NUMBER | `8` | Video quality (1-10, higher = better) |
| `--width` | PIXELS | `0` | Video width (0 = screen resolution) |
| `--height` | PIXELS | `0` | Video height (0 = screen resolution) |

#### Audio
| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `-a, --audio` | SOURCE | `system` | Audio source: `none`, `system`, `mic`, or `both` |
| `--no-audio` | FLAG | - | Shorthand for `--audio none` |

#### Display Selection
| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--display` | NUMBER | `0` | Display to capture (0 = primary) |
| `--monitor-switch-interval` | SECONDS | `1.0` | Check interval for multi-monitor switching |

#### Interaction Tracking
| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--track-interactions` | FLAG | - | Enable interaction tracking (always-on mode only) |
| `--track-mouse-moves` | FLAG | - | Track mouse movements (high data volume) |

#### Recording Type & Chunking
| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--recording-type` | TYPE | `always_on` | Recording type: `task` or `always_on` |
| `--task-id` | STRING | - | Task ID (required when `--recording-type task`) |
| `--is-final` | FLAG | - | Concatenate chunks (task mode only) |
| `--chunk-duration` | SECONDS | `10` | Duration of each video chunk |

#### Advanced
| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--ffmpeg-path` | PATH | `ffmpeg` | Path to custom ffmpeg binary |

## Output Files

### Always-On Mode Output Structure
```
~/.omega/
├── db.sqlite                           # SQLite database
└── data/
    └── always_on/
        ├── 2025-01-14_10-30-00.mp4    # Video chunk 1
        ├── 2025-01-14_10-30-10.mp4    # Video chunk 2
        └── interactions.json           # (if --track-interactions used)
```

### Task Mode Output Structure
```
~/.omega/
├── db.sqlite                           # SQLite database
└── data/
    └── tasks/
        └── my-task-123/
            ├── 2025-01-14_10-30-00.mp4       # Chunk 1
            ├── 2025-01-14_10-30-10.mp4       # Chunk 2
            ├── clicks.jsonl                   # Click events (always created)
            ├── interactions.json              # (if --track-interactions used)
            ├── final.mp4                      # (created when --is-final used)
            └── my-task-123_frames.json        # Frame metadata (created with final)
```

### Database Schema

The SQLite database (`~/.omega/db.sqlite`) contains:

**video_chunks table:**
- `id`: Primary key
- `file_path`: Path to video chunk
- `device_name`: Recording device hostname
- `recording_type`: "task" or "always_on"
- `task_id`: Associated task ID
- `chunk_index`: Sequential chunk number
- `created_at`: Timestamp

**frames table:**
- `id`: Primary key
- `video_chunk_id`: Foreign key to video_chunks
- `offset_index`: Frame index within chunk
- `timestamp`: Frame capture timestamp
- `is_keyframe`: 1 if keyframe, 0 otherwise
- `pts`: Presentation timestamp
- `dts`: Decode timestamp
- `display_index`: Which monitor was captured
- `display_width`: Display resolution width
- `display_height`: Display resolution height

### Click Events (clicks.jsonl)

Each line is a JSON object representing a click:

```json
{"x":450,"y":320,"button":"left","taskId":"my-task-123","timestamp":"2025-01-14T10:30:15.234Z","processName":"Chrome","windowTitle":"Example Page"}
{"x":680,"y":420,"button":"left","taskId":"my-task-123","timestamp":"2025-01-14T10:30:18.567Z","processName":"VSCode","windowTitle":"main.rs"}
```

**Note:** On macOS, `processName` and `windowTitle` require Accessibility permissions:
- System Settings → Privacy & Security → Accessibility → Add terminal/app

### Interaction Data (interactions.json)

Complete mouse and keyboard event history:

```json
{
  "duration_ms": 60000,
  "screen_width": 1920,
  "screen_height": 1080,
  "mouse_events": [
    {
      "timestamp_ms": 1234,
      "x": 450.5,
      "y": 320.2,
      "event_type": "click",
      "button": "left"
    },
    ...
  ],
  "keyboard_events": [
    {
      "timestamp_ms": 2345,
      "key": "A",
      "event_type": "press"
    },
    ...
  ],
  "metadata": {
    "started_at": "2025-01-14T10:30:00Z",
    "total_mouse_moves": 1234,
    "total_mouse_clicks": 45,
    "total_keyboard_events": 678
  }
}
```

### Frame Metadata (task-id_frames.json)

Created when using `--is-final` in task mode:

```json
{
  "task_id": "my-task-123",
  "final_video": "final.mp4",
  "total_frames": 1800,
  "normalized": true,
  "frames": [
    {
      "offset": 0,
      "timestamp": "2025-01-14T10:30:00.123Z",
      "pts": 0,
      "is_keyframe": true,
      "display_index": 0,
      "display_width": 1920,
      "display_height": 1080
    },
    ...
  ]
}
```

## Usage Examples

### Basic Recording

```bash
# Record for 30 seconds with default settings
screenrec record --duration 30

# Record with no audio
screenrec record --duration 60 --no-audio

# Record with microphone only
screenrec record --duration 60 --audio mic

# Unlimited recording (press Ctrl+C to stop)
screenrec record
```

### Custom Quality & Resolution

```bash
# High quality recording at 60fps
screenrec record --duration 60 --fps 60 --quality 10

# Low quality for testing (smaller file size)
screenrec record --duration 30 --fps 15 --quality 3

# Custom resolution
screenrec record --duration 60 --width 1280 --height 720
```

### Multi-Monitor Recording

```bash
# Record specific display
screenrec record --display 1 --duration 60

# Multi-monitor with faster switching detection (0.5 second interval)
screenrec record --duration 60 --monitor-switch-interval 0.5

# Note: With 2+ displays, cursor position automatically determines active display
```

### Interaction Tracking

```bash
# Track clicks and keyboard (always-on mode)
screenrec record --duration 60 --track-interactions

# Track clicks AND mouse movements (generates more data)
screenrec record --duration 60 --track-interactions --track-mouse-moves

# Task mode automatically tracks clicks to JSONL
screenrec record --recording-type task --task-id demo --duration 60
```

### Task-Based Recording

```bash
# Simple task recording
screenrec record \
  --recording-type task \
  --task-id user-onboarding \
  --duration 120

# Task recording with all features
screenrec record \
  --recording-type task \
  --task-id demo-2025 \
  --duration 60 \
  --fps 30 \
  --quality 9 \
  --audio both \
  --track-interactions \
  --track-mouse-moves

# Final recording session (concatenates all chunks)
screenrec record \
  --recording-type task \
  --task-id demo-2025 \
  --duration 60 \
  --is-final

# Custom output directory
screenrec record \
  --recording-type task \
  --task-id my-project \
  --output ~/Videos/my-project \
  --duration 60
```

### Custom Chunk Duration

```bash
# 30-second chunks instead of default 10-second
screenrec record --duration 300 --chunk-duration 30

# 5-second chunks for testing
screenrec record --duration 60 --chunk-duration 5
```

### Verbose Logging

```bash
# Enable debug logs for troubleshooting
screenrec --verbose record --duration 30

# Verbose screenshot
screenrec --verbose screenshot --output test.png
```

### Using Custom FFmpeg

```bash
# Specify custom ffmpeg binary path
screenrec record \
  --duration 60 \
  --ffmpeg-path /usr/local/bin/ffmpeg-custom
```

## Tips & Best Practices

### Performance Optimization
1. **FPS**: Use 30 FPS for most recordings. Lower FPS (15-24) for tutorials, higher (60) for gaming
2. **Quality**: Level 8 is a good balance. Use 9-10 for archival, 5-7 for sharing
3. **Resolution**: Let the tool auto-detect unless you need a specific size
4. **Chunk Duration**: 10-30 seconds is optimal. Too short = overhead, too long = memory usage

### Multi-Monitor Tips
- System automatically follows cursor between displays
- Each display can have different resolutions
- Use `--monitor-switch-interval 0.5` for faster switching if you frequently move between screens
- Final video is normalized to maximum resolution when using `--is-final`

### Task Mode Best Practices
1. **Use consistent task-id**: All recordings for same task should use same ID
2. **Concatenate at end**: Only use `--is-final` on the last recording session
3. **Check clicks.jsonl**: Verify click tracking is working during recording
4. **Grant Accessibility**: For meaningful `processName`/`windowTitle` on macOS

### Audio Considerations
- **System audio**: Captures application sounds (music, videos, notifications)
- **Microphone**: Captures your voice and ambient sounds
- **Both**: Records commentary over system audio
- **None**: Best for silent tutorials or when audio isn't needed

### Storage Management
- Video chunks are stored indefinitely until manually deleted
- SQLite database grows with frame metadata (negligible for most use cases)
- Use task mode for organized project-based storage
- Clean up `~/.omega/data/` periodically

### Interaction Tracking
- Click tracking has minimal overhead
- Mouse movement tracking generates significant data
- Use `--track-mouse-moves` only when needed for detailed playback
- Interaction JSON is useful for analytics and automation

### Troubleshooting
1. **Permission denied errors**: Grant Screen Recording permission in System Settings
2. **Accessibility warnings**: Grant Accessibility permission for window info capture
3. **FFmpeg not found**: Install FFmpeg v7 via package manager
4. **High CPU usage**: Lower FPS or quality setting
5. **Cursor not visible**: Ensure cursor tracking is enabled (automatic in most cases)

### Quality vs. File Size Reference

| Quality Level | CRF | Use Case | Approx. Size (1min @ 1080p30) |
|--------------|-----|----------|-------------------------------|
| 1-2 | 35-39 | Low quality tests | ~10-15 MB |
| 3-5 | 27-33 | Drafts, quick shares | ~20-40 MB |
| 6-8 | 18-24 | Standard recordings | ~50-100 MB |
| 9-10 | 12-15 | High quality archival | ~150-250 MB |

## Getting Help

```bash
# Show help for all commands
screenrec --help

# Show help for specific command
screenrec record --help
screenrec screenshot --help

# Enable verbose mode for debugging
screenrec --verbose record --duration 10
```

For issues or feature requests, visit: https://github.com/omegalabsinc/omega-screen-recorder
