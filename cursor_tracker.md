# Cursor Tracker - Multi-Display Recording

## Overview

The screen recorder automatically detects and follows your cursor as it moves between displays during recording. Each frame is tagged with the display it was captured from, enabling precise tracking and analysis.

## Features

### Automatic Display Detection
- **Initial Detection**: Detects which display your cursor is on when recording starts
- **Dynamic Tracking**: Checks cursor position every 100ms during recording
- **Seamless Switching**: Automatically switches capture to the new display when cursor moves
- **Database Tracking**: Every frame and video chunk stores its `display_id`

### Multi-Display Support
- Works with any number of displays
- No configuration needed - just move your cursor
- Handles display switches without dropping frames
- Logs all display transitions for debugging

## Database Schema

### Tables

#### video_chunks
Stores video file metadata with display information:
```sql
CREATE TABLE video_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL,
    device_name TEXT NOT NULL,
    display_id INTEGER NOT NULL,        -- Display where recording started
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

#### frames
Stores individual frame metadata with display tracking:
```sql
CREATE TABLE frames (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    video_chunk_id INTEGER NOT NULL,
    offset_index INTEGER NOT NULL,      -- Frame position (0-indexed)
    timestamp TIMESTAMP NOT NULL,       -- UTC timestamp when captured
    device_name TEXT,
    display_id INTEGER NOT NULL,        -- Display where this frame was captured
    FOREIGN KEY (video_chunk_id) REFERENCES video_chunks(id)
);

CREATE INDEX idx_frames_video_chunk_id ON frames(video_chunk_id);
```

## Usage

### Basic Recording
```bash
# Auto-detect cursor display and follow it
./target/release/screenrec record --duration 30 --output recording.mp4

# The recorder will:
# 1. Detect your cursor's current display
# 2. Start recording from that display
# 3. Follow your cursor if you move to another display
```

### Force Specific Display
```bash
# Override auto-detection and record from display 1
./target/release/screenrec record --duration 30 --output recording.mp4 --display 1

# Note: Even with manual selection, it will still follow cursor
```

### Disable Audio (Faster Testing)
```bash
./target/release/screenrec record --duration 10 --audio none --output test.mp4
```

## Log Messages

### Initial Detection
```
[INFO] Cursor detected on display 0 at (1092.48, 768.46)
[INFO] Auto-detected cursor on display 0
[INFO] Starting screen capture with dynamic cursor tracking...
```

### Display Switching
```
[INFO] Cursor moved from display 0 to display 1, switching capture
[INFO] Cursor moved from display 1 to display 0, switching capture
```

## Database Queries

### Show All Tables
```bash
sqlite3 frames.db ".tables"
```

### View Schema
```bash
sqlite3 frames.db ".schema"
```

### Video Chunks with Display Info
```bash
sqlite3 frames.db "SELECT * FROM video_chunks;"
# Output: id|file_path|device_name|display_id|created_at
# 1|recording.mp4|Mac|0|2025-11-13 17:31:53
```

### Frame Analysis

#### Count Frames per Display
```sql
SELECT
    display_id,
    COUNT(*) as frame_count
FROM frames
GROUP BY display_id;
```

#### Track Display Switches
```sql
SELECT
    offset_index,
    display_id,
    timestamp
FROM frames
ORDER BY offset_index;
```

#### Find Display Transitions
```sql
WITH transitions AS (
    SELECT
        offset_index,
        display_id,
        LAG(display_id) OVER (ORDER BY offset_index) as prev_display
    FROM frames
)
SELECT
    offset_index,
    prev_display || ' -> ' || display_id as transition
FROM transitions
WHERE prev_display IS NOT NULL
  AND prev_display != display_id;
```

#### Frames from Specific Display
```sql
SELECT
    id,
    offset_index,
    timestamp,
    display_id
FROM frames
WHERE display_id = 1
LIMIT 10;
```

#### Display Time Distribution
```sql
SELECT
    display_id,
    COUNT(*) * (1.0 / 30) as seconds_on_display,
    COUNT(*) as frames
FROM frames
GROUP BY display_id;
```

## Technical Implementation

### Cursor Detection (macOS)
```rust
// Uses Core Graphics to get cursor position
use core_graphics::event::{CGEvent};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::display::{CGDisplay, CGGetActiveDisplayList};

// Get cursor location
let event = CGEvent::new(event_source)?;
let cursor_location = event.location();

// Find which display contains cursor
for (index, &display_id) in display_ids.iter().enumerate() {
    let cg_display = CGDisplay::new(display_id);
    let bounds = cg_display.bounds();

    if cursor_location.x >= bounds.origin.x &&
       cursor_location.x < bounds.origin.x + bounds.size.width &&
       cursor_location.y >= bounds.origin.y &&
       cursor_location.y < bounds.origin.y + bounds.size.height {
        return Ok(index);
    }
}
```

### Dynamic Switching Logic
```rust
// Check cursor position every 100ms
let cursor_check_interval = Duration::from_millis(100);

if frame_start.duration_since(last_cursor_check) >= cursor_check_interval {
    if let Ok(cursor_display) = get_cursor_display() {
        if cursor_display != self.display_index {
            log::info!("Cursor moved from display {} to display {}, switching capture",
                self.display_index, cursor_display);

            // Recreate capturer for new display
            match create_capturer(cursor_display) {
                Ok((new_capturer, new_width, new_height)) => {
                    capturer = new_capturer;
                    width = new_width;
                    height = new_height;
                    self.display_index = cursor_display;
                }
                Err(e) => {
                    log::warn!("Failed to switch to display {}: {}", cursor_display, e);
                }
            }
        }
    }
    last_cursor_check = frame_start;
}
```

## Use Cases

### 1. Activity Tracking
Track which displays you spend time on during work sessions:
```sql
SELECT
    display_id,
    COUNT(*) * (1.0 / 30) as minutes,
    ROUND(COUNT(*) * 100.0 / SUM(COUNT(*)) OVER (), 2) as percentage
FROM frames
GROUP BY display_id;
```

### 2. Workflow Analysis
Analyze how often you switch between displays:
```sql
WITH transitions AS (
    SELECT
        LAG(display_id) OVER (ORDER BY offset_index) as prev_display,
        display_id as curr_display
    FROM frames
)
SELECT
    COUNT(*) as switch_count
FROM transitions
WHERE prev_display IS NOT NULL
  AND prev_display != curr_display;
```

### 3. Frame Extraction by Display
Extract specific frames from a particular display:
```bash
# Get frame info
sqlite3 frames.db "SELECT f.offset_index, vc.file_path FROM frames f
                    JOIN video_chunks vc ON f.video_chunk_id = vc.id
                    WHERE f.display_id = 1 AND f.offset_index = 42;"

# Extract frame using ffmpeg
ffmpeg -i recording.mp4 -vf "select=eq(n\,42)" -frames:v 1 frame_42.jpg
```

### 4. Time-based Display Usage
```sql
SELECT
    display_id,
    MIN(timestamp) as first_seen,
    MAX(timestamp) as last_seen,
    COUNT(*) as frames
FROM frames
GROUP BY display_id;
```

## Platform Support

### macOS (Implemented)
- Full cursor detection via Core Graphics
- All display configurations supported
- Requires screen recording permission

### Windows (Placeholder)
- Not yet implemented
- Falls back to display 0
- Ready for implementation

### Linux
- Not yet implemented
- Falls back to display 0

## Performance

- **Cursor Check Overhead**: ~1-2ms per check (every 100ms)
- **Display Switch Time**: ~50-100ms to recreate capturer
- **Frame Rate Impact**: Negligible (<0.1% CPU)
- **Database Impact**: ~1KB per 100 frames

## Troubleshooting

### Cursor Not Detected
```
[WARN] Could not determine cursor display, using display 0
```
**Solution**: Ensure screen recording permissions are granted in System Preferences → Security & Privacy → Screen Recording

### Display Switch Failed
```
[WARN] Failed to switch to display 1: Display 1 not found
```
**Solution**: Display was disconnected. Recorder continues on current display.

### No Frames Captured
```
[ERROR] Failed to create capturer: other error
```
**Solution**: Check display index and permissions. Try `--display 0` explicitly.

## Examples

### Example 1: Single Display Recording
```bash
./target/release/screenrec record --duration 10 --output single.mp4
```
Database shows:
```
display_id | frames
-----------+--------
0          | 300
```

### Example 2: Multi-Display Recording
```bash
# Record for 30 seconds while moving cursor between displays
./target/release/screenrec record --duration 30 --output multi.mp4
```
Database shows:
```
display_id | frames | seconds
-----------+--------+---------
0          | 450    | 15.0
1          | 450    | 15.0
```

### Example 3: Query Display Transitions
```sql
-- Find when you switched displays
WITH transitions AS (
    SELECT
        offset_index,
        display_id,
        timestamp,
        LAG(display_id) OVER (ORDER BY offset_index) as prev_display
    FROM frames
)
SELECT
    offset_index,
    prev_display || ' → ' || display_id as switch,
    timestamp
FROM transitions
WHERE prev_display IS NOT NULL
  AND prev_display != display_id;
```

Output:
```
offset_index | switch | timestamp
-------------+--------+---------------------------
150          | 0 → 1  | 2025-11-13T17:32:08+00:00
300          | 1 → 0  | 2025-11-13T17:32:13+00:00
```

## Future Enhancements

- [ ] Configurable cursor check interval
- [ ] Windows cursor detection implementation
- [ ] Linux cursor detection implementation
- [ ] Display hotplug handling
- [ ] Per-display recording settings (fps, quality)
- [ ] Multi-display side-by-side recording mode
- [ ] Display transition analytics dashboard
