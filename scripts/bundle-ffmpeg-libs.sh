#!/bin/bash
set -e

# Script to bundle FFmpeg dylibs with the omgrec binary for distribution
# This ensures the binary can run on systems without FFmpeg installed

BINARY_PATH="$1"
if [ -z "$BINARY_PATH" ]; then
    echo "Usage: $0 <path-to-binary> [arch]"
    echo "Example: $0 ./target/release/omgrec arm64"
    echo "Example: $0 ./target/x86_64-apple-darwin/release/omgrec x86_64"
    exit 1
fi

if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at $BINARY_PATH"
    exit 1
fi

# Detect architecture from binary or use provided argument
ARCH="$2"
if [ -z "$ARCH" ]; then
    # Auto-detect architecture from binary
    BINARY_ARCH=$(file "$BINARY_PATH" | grep -o 'arm64\|x86_64')
    if [ -z "$BINARY_ARCH" ]; then
        echo "Error: Could not detect architecture. Please specify: arm64 or x86_64"
        exit 1
    fi
    ARCH="$BINARY_ARCH"
fi

echo "Bundling FFmpeg libraries for $BINARY_PATH (architecture: $ARCH)..."

# Create architecture-specific lib directory next to the binary
BINARY_DIR=$(dirname "$BINARY_PATH")
LIB_DIR="$BINARY_DIR/lib-$ARCH"
mkdir -p "$LIB_DIR"

# List of FFmpeg libraries to bundle
FFMPEG_LIBS=(
    "libavutil.59.dylib"
    "libavformat.61.dylib"
    "libavfilter.10.dylib"
    "libavdevice.61.dylib"
    "libswscale.8.dylib"
    "libswresample.5.dylib"
    "libavcodec.61.dylib"
)

# Find FFmpeg library path
FFMPEG_LIB_PATH=""
if [ -d "/opt/homebrew/opt/ffmpeg@7/lib" ]; then
    FFMPEG_LIB_PATH="/opt/homebrew/opt/ffmpeg@7/lib"
elif [ -d "/usr/local/opt/ffmpeg@7/lib" ]; then
    FFMPEG_LIB_PATH="/usr/local/opt/ffmpeg@7/lib"
else
    echo "Error: FFmpeg libraries not found in expected Homebrew locations"
    exit 1
fi

echo "Found FFmpeg libraries at: $FFMPEG_LIB_PATH"

# Copy FFmpeg libraries
for lib in "${FFMPEG_LIBS[@]}"; do
    if [ -f "$FFMPEG_LIB_PATH/$lib" ]; then
        echo "Copying $lib..."
        # Remove existing file if it exists and make it writable
        if [ -f "$LIB_DIR/$lib" ]; then
            chmod +w "$LIB_DIR/$lib" 2>/dev/null || true
            rm -f "$LIB_DIR/$lib"
        fi
        # Copy library and make it writable so we can modify it
        cp "$FFMPEG_LIB_PATH/$lib" "$LIB_DIR/"
        chmod +w "$LIB_DIR/$lib"
        # Update library ID to use @rpath
        install_name_tool -id "@rpath/$lib" "$LIB_DIR/$lib"
    else
        echo "Warning: $lib not found"
    fi
done

# Update binary to use @rpath and set rpath to look in ./lib directory
echo "Updating binary to use @rpath..."
for lib in "${FFMPEG_LIBS[@]}"; do
    # Change absolute path to @rpath
    install_name_tool -change "$FFMPEG_LIB_PATH/$lib" "@rpath/$lib" "$BINARY_PATH" 2>/dev/null || true
done

# Add rpath to look in architecture-specific lib directory relative to binary
install_name_tool -add_rpath "@executable_path/lib-$ARCH" "$BINARY_PATH" 2>/dev/null || true
install_name_tool -add_rpath "@loader_path/lib-$ARCH" "$BINARY_PATH" 2>/dev/null || true

# Re-sign the binary after modifying it (required on macOS)
echo "Re-signing binary..."
codesign --force --sign - "$BINARY_PATH" 2>/dev/null || {
    echo "Warning: Could not re-sign binary. It may not execute on macOS."
}

echo "✓ Successfully bundled FFmpeg libraries"
echo "Libraries copied to: $LIB_DIR"
echo ""
echo "To verify, run: otool -L $BINARY_PATH"
echo "You should see @rpath references for FFmpeg libraries"
