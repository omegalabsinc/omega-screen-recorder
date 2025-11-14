# Build and Release Guide

## Quick Build

```bash
# Build locally
cargo build --release

# Binary at: target/release/omgrec
./target/release/omgrec --version
```

## Package with FFmpeg

```bash
# Create self-contained package with bundled FFmpeg
./scripts/package-with-ffmpeg.sh

# Output: target/omgrec-[platform]-bundled.tar.gz
# Contains: omgrec binary + lib/ folder with FFmpeg
```

## Release Process

### Create a Release

```bash
# 1. Tag version
git tag v0.1.0

# 2. Push tag
git push origin v0.1.0
```

GitHub Actions automatically:
- Builds for macOS ARM, macOS Intel, and Linux
- Bundles FFmpeg libraries with each binary
- Creates GitHub Release
- Uploads 3 packages:
  - `omgrec-macos-arm64.tar.gz`
  - `omgrec-macos-x86_64.tar.gz`
  - `omgrec-linux-x86_64.tar.gz`

### User Installation

```bash
# Download
curl -L -O https://github.com/YOUR_ORG/omega-screen-recorder/releases/download/v0.1.0/omgrec-macos-arm64.tar.gz

# Extract
tar -xzf omgrec-macos-arm64.tar.gz

# Run (no FFmpeg install needed!)
./omgrec --version
./omgrec record --duration 60
```

## What Gets Bundled

Each package includes:
- `omgrec` binary (~5 MB)
- `lib/` folder with FFmpeg libraries (~30 MB)
- `README.txt`

**Total size:** ~35 MB per platform

Users don't need to install FFmpeg - it's all bundled!

## Technical Details

### macOS
- Uses `@rpath` to find libraries in `./lib/`
- No environment variables needed
- Works from any directory

### Linux
- Uses `$ORIGIN/lib` in RPATH
- Requires `patchelf` for building
- No environment variables needed

### Verification

```bash
# macOS: Check @rpath
otool -L omgrec | grep @rpath

# Linux: Check RPATH
patchelf --print-rpath omgrec

# Test without system FFmpeg
brew uninstall ffmpeg  # or apt-get remove ffmpeg
./omgrec --version     # Should still work!
```

## Default Settings

- Resolution: 1280x720 (optimized for performance)
- FPS: 30
- Quality: 8/10
- Chunk duration: 10 seconds

Override with flags:
```bash
omgrec record --fps 15 --quality 6 --width 1920 --height 1080
```

That's it! 🎉
