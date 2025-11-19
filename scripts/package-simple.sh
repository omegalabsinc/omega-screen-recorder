#!/bin/bash
set -e

# Simple packaging script without FFmpeg bundling
# Users must have FFmpeg installed globally or provide --ffmpeg-path argument

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}📦 Packaging omgrec (without FFmpeg bundling)${NC}"

# Detect platform and architecture
OS=$(uname -s)
ARCH=$(uname -m)

if [ "$OS" == "Darwin" ]; then
    PLATFORM="macos"
    if [ "$ARCH" == "arm64" ]; then
        TARGET="aarch64-apple-darwin"
        PLATFORM_NAME="macos-arm64"
        FFMPEG_PREFIX="${FFMPEG_PREFIX:-/opt/homebrew}"
    else
        TARGET="x86_64-apple-darwin"
        PLATFORM_NAME="macos-x86_64"
        FFMPEG_PREFIX="${FFMPEG_PREFIX:-/usr/local}"
    fi
elif [ "$OS" == "Linux" ]; then
    PLATFORM="linux"
    TARGET="x86_64-unknown-linux-gnu"
    PLATFORM_NAME="linux-x86_64"
    FFMPEG_PREFIX="${FFMPEG_PREFIX:-/usr}"
else
    echo -e "${RED}❌ Unsupported platform: $OS${NC}"
    exit 1
fi

echo -e "${YELLOW}Platform: $PLATFORM_NAME${NC}"
echo -e "${YELLOW}Target: $TARGET${NC}"
echo -e "${YELLOW}FFmpeg Prefix (for build only): $FFMPEG_PREFIX${NC}"

# Ensure FFmpeg environment variables are set for the build
export FFMPEG_DIR="${FFMPEG_DIR:-$FFMPEG_PREFIX}"
export FFMPEG_INCLUDE_DIR="${FFMPEG_INCLUDE_DIR:-$FFMPEG_PREFIX/include}"
export FFMPEG_LIB_DIR="${FFMPEG_LIB_DIR:-$FFMPEG_PREFIX/lib}"

# Also set these for ffmpeg-sys-next
export DEP_FFMPEG_INCLUDE="$FFMPEG_INCLUDE_DIR"
export DEP_FFMPEG_LIB="$FFMPEG_LIB_DIR"

echo -e "${YELLOW}FFMPEG_DIR: $FFMPEG_DIR${NC}"
echo -e "${YELLOW}FFMPEG_INCLUDE_DIR: $FFMPEG_INCLUDE_DIR${NC}"
echo -e "${YELLOW}FFMPEG_LIB_DIR: $FFMPEG_LIB_DIR${NC}"

# Verify FFmpeg headers exist before building
if [ ! -f "$FFMPEG_INCLUDE_DIR/libavcodec/avcodec.h" ]; then
    echo -e "${RED}❌ FFmpeg headers not found at $FFMPEG_INCLUDE_DIR${NC}"
    echo -e "${RED}   Looking for: $FFMPEG_INCLUDE_DIR/libavcodec/avcodec.h${NC}"
    echo -e "${YELLOW}   Please ensure FFmpeg is installed for building${NC}"

    if [ "$PLATFORM" == "macos" ]; then
        echo -e "${YELLOW}   On macOS: brew install ffmpeg${NC}"
    elif [ "$PLATFORM" == "linux" ]; then
        echo -e "${YELLOW}   On Ubuntu/Debian: sudo apt-get install libavcodec-dev libavformat-dev libavutil-dev${NC}"
    fi
    exit 1
fi

echo -e "${GREEN}✅ FFmpeg headers found${NC}"

# Build the binary
echo -e "\n${GREEN}🔨 Building release binary...${NC}"
cd "$PROJECT_ROOT"

cargo build --release --target "$TARGET"

BINARY_PATH="$PROJECT_ROOT/target/$TARGET/release/omgrec"

if [ ! -f "$BINARY_PATH" ]; then
    echo -e "${RED}❌ Binary not found at $BINARY_PATH${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Binary built successfully${NC}"

# Create package directory
PACKAGE_DIR="$PROJECT_ROOT/target/package-$PLATFORM_NAME"
rm -rf "$PACKAGE_DIR"
mkdir -p "$PACKAGE_DIR"

echo -e "\n${GREEN}📋 Copying binary...${NC}"
cp "$BINARY_PATH" "$PACKAGE_DIR/omgrec"

# Create README
echo -e "\n${GREEN}📄 Creating README...${NC}"
cat > "$PACKAGE_DIR/README.txt" << 'README_EOF'
omgrec - Omega Screen Recorder

This package includes ONLY the omgrec binary. FFmpeg is NOT bundled.

Requirements:
- FFmpeg must be installed on your system, OR
- Provide the --ffmpeg-path argument when running omgrec

Installation:

1. Install FFmpeg:

   macOS:
     brew install ffmpeg

   Ubuntu/Debian:
     sudo apt-get install ffmpeg

   Or download from: https://ffmpeg.org/download.html

2. Extract this archive:
   tar -xzf omgrec-*.tar.gz

3. Grant Screen Recording permission (macOS only):
   The binary requires Screen Recording permission to capture your screen.

   Since this binary is unsigned, macOS will NOT prompt for permission.
   Instead, you need to grant permission to your Terminal app:

   a. Go to: System Settings > Privacy & Security > Screen Recording
   b. Click the (+) button or enable your terminal app:
      - Terminal.app (if using macOS Terminal)
      - iTerm (if using iTerm2)
      - Code (if using VS Code's integrated terminal)
   c. Restart your terminal app after granting permission

4. Run omgrec:

   With system FFmpeg:
     ./omgrec record --duration 10
     ./omgrec screenshot --output screenshot.png

   With custom FFmpeg path:
     ./omgrec record --duration 10 --ffmpeg-path /path/to/ffmpeg

5. (Optional) Move to PATH:
   sudo cp omgrec /usr/local/bin/

Important Notes:
- FFmpeg is NOT bundled - you must install it separately
- Use --ffmpeg-path to specify a custom FFmpeg location
- On macOS: Grant Screen Recording permission to your Terminal app
- The binary will automatically detect system-installed FFmpeg if available

For more information, visit:
https://github.com/OmegaLabs/omega-screen-recorder
README_EOF

# Create tarball
echo -e "\n${GREEN}📦 Creating tarball...${NC}"
cd "$PROJECT_ROOT/target"
TARBALL="omgrec-$PLATFORM_NAME.tar.gz"
tar -czf "$TARBALL" -C "package-$PLATFORM_NAME" .

TARBALL_PATH="$PROJECT_ROOT/target/$TARBALL"
TARBALL_SIZE=$(du -h "$TARBALL_PATH" | awk '{print $1}')

echo -e "\n${GREEN}✅ Package created successfully!${NC}"
echo -e "${YELLOW}Location: $TARBALL_PATH${NC}"
echo -e "${YELLOW}Size: $TARBALL_SIZE${NC}"

echo -e "\n${GREEN}🎉 Done! Extract and test the package:${NC}"
echo -e "${YELLOW}  cd /tmp${NC}"
echo -e "${YELLOW}  tar -xzf $TARBALL_PATH${NC}"
echo -e "${YELLOW}  ./omgrec --help${NC}"
echo -e "${YELLOW}  ${NC}"
echo -e "${YELLOW}  Remember: FFmpeg must be installed on the system!${NC}"
