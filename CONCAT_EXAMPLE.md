# Concatenation Example - Task "abc"

## Summary

Successfully concatenated video chunks for task ID: **abc**

## Command Used

```bash
./target/release/omgrec concat --task-id abc
```

## Progress Output

```
🔄 [PROGRESS] Starting concatenation for task: abc
🔄 [PROGRESS] Validating FFmpeg installation...
✅ [PROGRESS] FFmpeg validated: ffmpeg version 7.1.2
🔄 [PROGRESS] Loading recording data from database...
✅ [PROGRESS] Found 246 video chunks to concatenate
🔄 [PROGRESS] Analyzing video frames and resolutions...
⚠️  [PROGRESS] Multiple resolutions detected - normalization required
📐 [PROGRESS] Target resolution: 2560x1440
🔄 [PROGRESS] Preparing concatenation list...
⚠️  [PROGRESS] Warning: 87 chunk files missing, using 159 available chunks
🎬 [PROGRESS] Starting FFmpeg concatenation...
   Output: /Users/pushkarborkar/.omega/data/tasks/abc/final.mp4
✅ [PROGRESS] Video concatenation complete!
✅ Final video saved to: /Users/pushkarborkar/.omega/data/tasks/abc/final.mp4
🔄 [PROGRESS] Extracting video metadata...
🔄 [PROGRESS] Calculating recording statistics...
🔄 [PROGRESS] Generating metadata files...
✅ [PROGRESS] Metadata file created
   📄 /Users/pushkarborkar/.omega/data/tasks/abc/metadata.json
🔄 [PROGRESS] Exporting frame-level data...
✅ [PROGRESS] Frame data exported (23781 frames)
   📄 /Users/pushkarborkar/.omega/data/tasks/abc/abc_frames.json

🎉 [PROGRESS] Concatenation complete!
   Duration: 792.6s | Size: 125MB | Frames: 23781
```

## Output Files

All files are in: `~/.omega/data/tasks/abc/`

### 1. final.mp4 (125 MB)
The concatenated video file:
- Combined 159 chunks (87 were missing from disk)
- Normalized from 2 resolutions (2560x1440 and 1728x1117) to 2560x1440
- Duration: ~13 minutes (792.6 seconds)
- 23,781 frames total

### 2. metadata.json (46 KB)
Comprehensive recording metadata including:
- Task information
- Video details (duration, size, codec, bitrate, fps)
- Chunk details (246 DB entries, 159 used)
- Frame statistics (23,781 total, keyframe info)
- Display information (resolutions, monitors)
- Recording settings

Sample structure:
```json
{
  "version": "1.0",
  "task_id": "abc",
  "device_name": "unknown",
  "recording_type": "task",
  "video": {
    "final_video_path": "final.mp4",
    "duration_seconds": 792.6,
    "duration_formatted": "0h 13m 12.6s",
    "file_size_bytes": 131072000,
    "file_size_mb": "125.00",
    "codec": "h264",
    "bitrate_bps": 1322600,
    "fps": 30,
    "quality": 8
  },
  "chunks": {
    "total_count": 246
  },
  "frames": {
    "total_count": 23781,
    "keyframe_count": 794,
    "keyframe_interval": 29
  },
  "displays": {
    "monitors_used": 2,
    "normalized": true,
    "resolutions": [
      {"width": 2560, "height": 1440, "frame_count": 22105},
      {"width": 1728, "height": 1117, "frame_count": 1676}
    ],
    "final_resolution": {"width": 2560, "height": 1440}
  }
}
```

### 3. abc_frames.json (4.9 MB)
Frame-by-frame data with timestamps:
- All 23,781 frames
- Timestamp, PTS, keyframe status
- Display index and resolution per frame

Sample frame entry:
```json
{
  "offset": 0,
  "timestamp": "2025-11-13T19:36:00.000Z",
  "pts": 0,
  "is_keyframe": true,
  "display_index": 0,
  "display_width": 2560,
  "display_height": 1440
}
```

## Key Features Demonstrated

### 1. Missing File Handling
- Database had 246 chunk references
- Only 159 files existed on disk
- System automatically skipped missing files
- Warning issued: `⚠️ Warning: 87 chunk files missing, using 159 available chunks`
- Concatenation continued successfully

### 2. Multi-Resolution Normalization
- Detected 2 different resolutions:
  - 2560x1440 (primary monitor)
  - 1728x1117 (secondary monitor)
- Automatically normalized to maximum: 2560x1440
- Applied FFmpeg scaling and padding filters
- Result: Consistent resolution throughout video

### 3. Progress Tracking
- 18 distinct progress messages
- Clear emoji indicators for status
- Percentage-trackable phases
- Error and warning notifications
- Final summary with stats

## Usage in Electron

```javascript
const { spawn } = require('child_process');

function concatenateTask(taskId) {
  const concat = spawn('omgrec', ['concat', '--task-id', taskId]);

  concat.stdout.on('data', (data) => {
    const output = data.toString();
    output.split('\n').forEach(line => {
      if (line.includes('[PROGRESS]')) {
        const message = line.split('[PROGRESS]')[1].trim();
        console.log(message);

        // Parse for specific events
        if (line.includes('🎉') && line.includes('complete')) {
          console.log('✅ Concatenation finished!');
        } else if (line.includes('⚠️')) {
          console.warn('Warning:', message);
        }
      }
    });
  });

  concat.on('exit', (code) => {
    if (code === 0) {
      console.log('Success! Files ready in ~/.omega/data/tasks/' + taskId);
    } else {
      console.error('Concatenation failed with code:', code);
    }
  });
}

// Run it
concatenateTask('abc');
```

## Notes

- **Resilient**: Handles missing files gracefully
- **Smart**: Detects and normalizes multi-resolution recordings
- **Fast**: For single-resolution recordings, uses stream copy (no re-encoding)
- **Complete**: Generates video + comprehensive metadata
- **Trackable**: Rich progress messages for UI integration
- **Safe**: Non-destructive (preserves original chunks)

## Testing Other Tasks

Check available tasks:
```bash
sqlite3 ~/.omega/db.sqlite "SELECT DISTINCT task_id FROM video_chunks"
```

Concatenate any task:
```bash
./target/release/omgrec concat --task-id <task-id>
```

With custom output path:
```bash
./target/release/omgrec concat --task-id <task-id> --output /path/to/output.mp4
```

With verbose logging:
```bash
./target/release/omgrec concat --task-id <task-id> --verbose
```
