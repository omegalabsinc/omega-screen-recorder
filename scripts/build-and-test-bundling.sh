#!/bin/bash
set -e

echo "================================"
echo "Building and Testing FFmpeg Bundling"
echo "================================"
echo ""

echo "Step 1: Building release binary..."
cargo build --release

echo ""
echo "Step 2: Bundling FFmpeg libraries..."
./scripts/bundle-ffmpeg-libs.sh ./target/release/omgrec

echo ""
echo "Step 3: Verifying bundling with otool..."
echo ""
echo "Binary FFmpeg library references:"
otool -L ./target/release/omgrec | grep -E "(avcodec|avformat|avutil|swscale|swresample|avfilter|avdevice)"

echo ""
echo "Binary rpath configuration:"
otool -l ./target/release/omgrec | grep -A 2 "LC_RPATH"

echo ""
echo "Bundled library install names (sample):"
echo "libavcodec.61.dylib:"
otool -D ./target/release/lib/libavcodec.61.dylib
echo "libavutil.59.dylib:"
otool -D ./target/release/lib/libavutil.59.dylib

echo ""
echo "Step 4: Running bundling tests..."
cargo test --test test_ffmpeg_bundling

echo ""
echo "✅ All bundling verification complete!"
echo ""
echo "Distribution structure:"
echo "  omgrec                  (binary with @rpath references)"
echo "  lib/                    (7 FFmpeg dylibs, ~16MB total)"
echo ""
echo "You can now package with:"
echo "  cd target/release && tar -czf omgrec-macos.tar.gz omgrec lib/"
