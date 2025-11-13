#!/bin/bash
# Screen Recording Conversion Script
# This script converts the captured frames to a video file

OUTPUT_FILE="../demo.mp4"
CRF=18
FFCONCAT_FILE="frames.ffconcat"

if [ ! -f "$FFCONCAT_FILE" ]; then
    echo "Error: $FFCONCAT_FILE not found"
    exit 1
fi

echo "Converting frames to video..."
echo "Output: $OUTPUT_FILE"

# Check if ffmpeg is installed
if ! command -v ffmpeg &> /dev/null; then
    echo "Error: ffmpeg is not installed"
    echo "Install with: brew install ffmpeg (macOS) or apt-get install ffmpeg (Linux)"
    exit 1
fi

ffmpeg -y -f concat -safe 0 -i "$FFCONCAT_FILE" \
    -vsync vfr -pix_fmt yuv420p \
    -c:v libx264 -preset medium -crf $CRF \
    "$OUTPUT_FILE"

if [ $? -eq 0 ]; then
    echo "✅ Video created: $OUTPUT_FILE"
else
    echo "❌ Conversion failed"
    exit 1
fi
