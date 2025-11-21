# Building Screenrec

## CI/CD Workflows

This project uses GitHub Actions to automatically build binaries for multiple platforms.

### Available Workflows

#### 1. Build Windows Binary (`build-windows.yml`)
- Triggers on: pushes to main, pull requests, tags, and manual dispatch
- Builds: Windows x86_64 binary
- Artifact: `screenrec-windows-x86_64`
- Creates GitHub releases when tagged with `v*`

#### 2. Multi-Platform Release (`release.yml`)
- Triggers on: version tags (`v*`) and manual dispatch
- Builds for:
  - Windows (x86_64)
  - macOS (x86_64 Intel)
  - macOS (aarch64 Apple Silicon)
  - Linux (x86_64)
- Creates GitHub releases with all platform binaries

### Getting Windows Binaries

#### Option 1: Download from Artifacts
1. Go to the [Actions tab](../../actions)
2. Click on the latest successful workflow run
3. Download the `screenrec-windows-x86_64` artifact

#### Option 2: Create a Release
1. Create and push a version tag:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
2. The workflow will automatically build and create a GitHub release
3. Download the Windows binary from the [Releases page](../../releases)

#### Option 3: Manual Trigger
1. Go to [Actions](../../actions)
2. Select "Build Windows Binary" workflow
3. Click "Run workflow"
4. Download the artifact once complete

### Local Cross-Compilation (Not Recommended)

Cross-compiling to Windows from macOS is complex due to FFmpeg dependencies. Using CI/CD is the recommended approach.

If you must build locally on macOS for testing:
```bash
# This requires complex setup and may not work for all dependencies
rustup target add x86_64-pc-windows-msvc
cargo build --target x86_64-pc-windows-msvc --release
```

Note: FFmpeg linking will likely fail without a Windows environment.

### Requirements

The workflows automatically install:
- Rust toolchain
- FFmpeg and required libraries
- Platform-specific dependencies

### Troubleshooting

If builds fail:
1. Check the Actions logs for specific errors
2. Ensure all dependencies are available on the target platform
3. Verify FFmpeg installation succeeded
4. Check that Windows-specific code in `Cargo.toml` is correct
