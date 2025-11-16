#!/bin/bash
set -e

# Script to package omgrec binary with FFmpeg libraries
# This creates a self-contained bundle that doesn't require FFmpeg installation

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}📦 Packaging omgrec with FFmpeg libraries${NC}"

# Detect platform and architecture
OS=$(uname -s)
ARCH=$(uname -m)

if [ "$OS" == "Darwin" ]; then
    PLATFORM="macos"
    if [ "$ARCH" == "arm64" ]; then
        TARGET="aarch64-apple-darwin"
        PLATFORM_NAME="macos-arm64"
        FFMPEG_PREFIX="/opt/homebrew"
    else
        TARGET="x86_64-apple-darwin"
        PLATFORM_NAME="macos-x86_64"
        FFMPEG_PREFIX="/usr/local"
    fi
    LIB_EXT="dylib"
elif [ "$OS" == "Linux" ]; then
    PLATFORM="linux"
    TARGET="x86_64-unknown-linux-gnu"
    PLATFORM_NAME="linux-x86_64"
    FFMPEG_PREFIX="/usr"
    LIB_EXT="so"
else
    echo -e "${RED}❌ Unsupported platform: $OS${NC}"
    exit 1
fi

echo -e "${YELLOW}Platform: $PLATFORM_NAME${NC}"
echo -e "${YELLOW}Target: $TARGET${NC}"

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
mkdir -p "$PACKAGE_DIR/lib"

echo -e "\n${GREEN}📋 Copying binary...${NC}"
cp "$BINARY_PATH" "$PACKAGE_DIR/omgrec"

# Find and copy FFmpeg libraries
echo -e "\n${GREEN}📚 Finding FFmpeg libraries...${NC}"

if [ "$PLATFORM" == "macos" ]; then
    # Get list of FFmpeg libraries the binary depends on
    FFMPEG_LIBS=$(otool -L "$BINARY_PATH" | grep -E "(libav|libsw)" | awk '{print $1}')

    echo -e "${YELLOW}FFmpeg libraries to bundle:${NC}"

    for lib in $FFMPEG_LIBS; do
        lib_name=$(basename "$lib")
        echo "  - $lib_name"

        # Copy the library
        if [ -f "$lib" ]; then
            cp "$lib" "$PACKAGE_DIR/lib/"
        else
            echo -e "${RED}    Warning: $lib not found${NC}"
        fi
    done

    # Also copy their dependencies (transitive dependencies)
    echo -e "\n${GREEN}📚 Finding transitive dependencies...${NC}"
    for lib in "$PACKAGE_DIR"/lib/*.dylib; do
        if [ -f "$lib" ]; then
            lib_deps=$(otool -L "$lib" | grep -E "(libav|libsw|libx264|libx265|libvpx)" | awk '{print $1}' | grep -v "@rpath")
            for dep in $lib_deps; do
                dep_name=$(basename "$dep")
                if [ ! -f "$PACKAGE_DIR/lib/$dep_name" ] && [ -f "$dep" ]; then
                    echo "  + $dep_name (transitive)"
                    cp "$dep" "$PACKAGE_DIR/lib/"
                fi
            done
        fi
    done

    # Modify the binary to use @rpath
    echo -e "\n${GREEN}🔧 Setting up @rpath for binary...${NC}"
    install_name_tool -add_rpath "@executable_path/lib" "$PACKAGE_DIR/omgrec"

    # Update library references in the binary
    for lib in $FFMPEG_LIBS; do
        lib_name=$(basename "$lib")
        if [ -f "$PACKAGE_DIR/lib/$lib_name" ]; then
            echo "  Updating reference: $lib_name"
            install_name_tool -change "$lib" "@rpath/$lib_name" "$PACKAGE_DIR/omgrec" 2>/dev/null || true
        fi
    done

    # Fix library interdependencies
    echo -e "\n${GREEN}🔧 Fixing library interdependencies...${NC}"
    for lib in "$PACKAGE_DIR"/lib/*.dylib; do
        lib_name=$(basename "$lib")
        echo "  Processing: $lib_name"

        # Set library ID
        install_name_tool -id "@rpath/$lib_name" "$lib" 2>/dev/null || true

        # Update references in this library to other bundled libraries
        lib_refs=$(otool -L "$lib" | grep -E "(libav|libsw)" | awk '{print $1}' | grep -v "@rpath")
        for ref in $lib_refs; do
            ref_name=$(basename "$ref")
            if [ -f "$PACKAGE_DIR/lib/$ref_name" ]; then
                install_name_tool -change "$ref" "@rpath/$ref_name" "$lib" 2>/dev/null || true
            fi
        done
    done

elif [ "$PLATFORM" == "linux" ]; then
    # For Linux, copy FFmpeg .so files
    FFMPEG_LIBS=$(ldd "$BINARY_PATH" | grep -E "(libav|libsw)" | awk '{print $3}')

    echo -e "${YELLOW}FFmpeg libraries to bundle:${NC}"

    for lib in $FFMPEG_LIBS; do
        lib_name=$(basename "$lib")
        echo "  - $lib_name"

        if [ -f "$lib" ]; then
            cp "$lib" "$PACKAGE_DIR/lib/"

            # Also copy the symlinks
            lib_base=$(echo "$lib_name" | sed 's/\.so\..*//')
            for symlink in $(ls "$lib" 2>/dev/null); do
                if [ -L "$symlink" ]; then
                    cp -P "$symlink" "$PACKAGE_DIR/lib/" 2>/dev/null || true
                fi
            done
        fi
    done

    # Set RPATH for Linux
    echo -e "\n${GREEN}🔧 Setting up RPATH for binary...${NC}"
    patchelf --set-rpath '$ORIGIN/lib' "$PACKAGE_DIR/omgrec" 2>/dev/null || {
        echo -e "${YELLOW}⚠️  patchelf not found. Install it: sudo apt-get install patchelf${NC}"
        echo -e "${YELLOW}⚠️  Creating wrapper script instead...${NC}"

        # Create wrapper script
        mv "$PACKAGE_DIR/omgrec" "$PACKAGE_DIR/omgrec-bin"
        cat > "$PACKAGE_DIR/omgrec" << 'WRAPPER_EOF'
#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export LD_LIBRARY_PATH="$SCRIPT_DIR/lib:$LD_LIBRARY_PATH"
exec "$SCRIPT_DIR/omgrec-bin" "$@"
WRAPPER_EOF
        chmod +x "$PACKAGE_DIR/omgrec"
    }
fi

# Verify the package
echo -e "\n${GREEN}🔍 Verifying package...${NC}"

if [ "$PLATFORM" == "macos" ]; then
    echo -e "${YELLOW}Binary dependencies:${NC}"
    otool -L "$PACKAGE_DIR/omgrec" | grep -E "(libav|libsw|@rpath)"

    echo -e "\n${YELLOW}Bundled libraries:${NC}"
    ls -lh "$PACKAGE_DIR/lib/"

elif [ "$PLATFORM" == "linux" ]; then
    echo -e "${YELLOW}Binary dependencies:${NC}"
    ldd "$PACKAGE_DIR/omgrec" | grep -E "(libav|libsw)"

    echo -e "\n${YELLOW}Bundled libraries:${NC}"
    ls -lh "$PACKAGE_DIR/lib/"
fi

# Create tarball
echo -e "\n${GREEN}📦 Creating tarball...${NC}"
cd "$PROJECT_ROOT/target"
TARBALL="omgrec-$PLATFORM_NAME-bundled.tar.gz"
tar -czf "$TARBALL" -C "package-$PLATFORM_NAME" .

TARBALL_PATH="$PROJECT_ROOT/target/$TARBALL"
TARBALL_SIZE=$(du -h "$TARBALL_PATH" | awk '{print $1}')

echo -e "\n${GREEN}✅ Package created successfully!${NC}"
echo -e "${YELLOW}Location: $TARBALL_PATH${NC}"
echo -e "${YELLOW}Size: $TARBALL_SIZE${NC}"

# Create README for the package
cat > "$PACKAGE_DIR/README.txt" << 'README_EOF'
omgrec - Omega Screen Recorder

This package includes the omgrec binary and all required FFmpeg libraries.
No additional FFmpeg installation is required!

Installation:
1. Extract this archive:
   tar -xzf omgrec-*.tar.gz

2. Run omgrec:
   ./omgrec --version
   ./omgrec record --duration 10

3. (Optional) Move to PATH:
   sudo cp omgrec /usr/local/bin/
   sudo cp -r lib /usr/local/lib/omgrec-libs/

For more information, visit:
https://github.com/OmegaLabs/omega-screen-recorder
README_EOF

echo -e "\n${GREEN}📄 Created README.txt in package${NC}"

# Test the package
echo -e "\n${GREEN}🧪 Testing packaged binary...${NC}"
cd "$PACKAGE_DIR"
./omgrec --version && echo -e "${GREEN}✅ Package test successful!${NC}" || echo -e "${RED}❌ Package test failed${NC}"

echo -e "\n${GREEN}🎉 Done! Extract and test the package:${NC}"
echo -e "${YELLOW}  cd /tmp${NC}"
echo -e "${YELLOW}  tar -xzf $TARBALL_PATH${NC}"
echo -e "${YELLOW}  ./omgrec --version${NC}"
