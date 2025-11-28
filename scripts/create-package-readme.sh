#!/bin/bash
# Create README for distribution packages

ARCH="$1"
OUTPUT_DIR="$2"

if [ -z "$ARCH" ] || [ -z "$OUTPUT_DIR" ]; then
    echo "Usage: $0 <arch> <output_dir>"
    echo "Example: $0 arm64 target/package-macos-arm64"
    exit 1
fi

cat > "$OUTPUT_DIR/README.txt" << 'EOF'
omgrec - Omega Screen Recorder

This package contains ONLY the omgrec binary. FFmpeg is NOT bundled.

Requirements:
- FFmpeg must be installed on your system, OR
- Provide the --ffmpeg-path argument when running omgrec

Installation:
1. Install FFmpeg: brew install ffmpeg
   Or download from: https://ffmpeg.org/download.html

2. Extract this archive

3. Grant Screen Recording permission to your Terminal app:
   System Settings > Privacy & Security > Screen Recording

   Note: Enable the Terminal app you're using (Terminal.app, iTerm, VS Code, etc.)

4. Run: ./omgrec record --duration 10

For custom FFmpeg location:
  ./omgrec record --duration 10 --ffmpeg-path /path/to/ffmpeg

Common Commands:
  ./omgrec record --duration 30          # Record for 30 seconds
  ./omgrec record --output my-video.mp4  # Record with custom output path
  ./omgrec screenshot --output test.png  # Take a screenshot
  ./omgrec list-displays                 # List available displays
  ./omgrec --help                        # Show all options

For more info: https://github.com/OmegaLabs/omega-screen-recorder
EOF

echo "✅ README created at $OUTPUT_DIR/README.txt"
