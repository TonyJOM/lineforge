#!/bin/bash
set -e

REPO="tonyjom/lineforge"
BINARY_NAME="forge"
INSTALL_DIR="${HOME}/.local/bin"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "Installing Lineforge..."
echo ""

# Parse arguments
FROM_SOURCE=false
for arg in "$@"; do
    case $arg in
        --from-source)
            FROM_SOURCE=true
            shift
            ;;
    esac
done

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$ARCH" in
    x86_64) ARCH="x86_64" ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *) echo -e "${RED}Error: Unsupported architecture: $ARCH${NC}"; exit 1 ;;
esac

# Map OS to target triple component
case "$OS" in
    linux) TARGET_OS="unknown-linux-gnu" ;;
    darwin) TARGET_OS="apple-darwin" ;;
    *) echo -e "${RED}Error: Unsupported OS: $OS${NC}"; exit 1 ;;
esac

TARGET="${ARCH}-${TARGET_OS}"
echo "Detected platform: $OS ($ARCH)"
echo "Target: $TARGET"
echo ""

mkdir -p "$INSTALL_DIR"

# Function to get latest release version
get_latest_version() {
    curl -sL "https://api.github.com/repos/${REPO}/releases/latest" | \
        grep '"tag_name":' | \
        sed -E 's/.*"([^"]+)".*/\1/'
}

# Function to download and install binary
download_binary() {
    local version=$1
    local target=$2
    local url="https://github.com/${REPO}/releases/download/${version}/${BINARY_NAME}-${version}-${target}.tar.gz"

    echo "Downloading from: $url"
    echo ""

    TEMP_DIR=$(mktemp -d)
    trap "rm -rf $TEMP_DIR" EXIT

    if curl -fsSL "$url" -o "$TEMP_DIR/${BINARY_NAME}.tar.gz"; then
        echo "Extracting..."
        tar -xzf "$TEMP_DIR/${BINARY_NAME}.tar.gz" -C "$TEMP_DIR"

        echo "Installing to $INSTALL_DIR..."
        cp "$TEMP_DIR/${BINARY_NAME}" "$INSTALL_DIR/"
        chmod +x "$INSTALL_DIR/${BINARY_NAME}"

        return 0
    else
        return 1
    fi
}

# Function to build from source
build_from_source() {
    if ! command -v cargo &> /dev/null; then
        echo -e "${RED}Error: cargo not found. Please install Rust first.${NC}"
        echo "Visit: https://rustup.rs"
        exit 1
    fi

    echo "Building from source with cargo..."
    echo ""

    # Check if we're in the repo already
    if [ -f "Cargo.toml" ] && grep -q 'name = "forge"' Cargo.toml 2>/dev/null; then
        echo "Building in current directory..."
        BUILD_DIR="."
    else
        # Clone if not in repo
        BUILD_DIR=$(mktemp -d)
        echo "Cloning repository to $BUILD_DIR..."
        git clone "https://github.com/${REPO}" "$BUILD_DIR"
        cd "$BUILD_DIR"
    fi

    # Build release binary
    echo "Building release binary..."
    cargo build --release

    # Copy binary
    echo "Installing binary to $INSTALL_DIR..."
    cp target/release/${BINARY_NAME} "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/${BINARY_NAME}"
}

# Main installation logic
if [ "$FROM_SOURCE" = true ]; then
    echo "Installing from source (--from-source flag)..."
    build_from_source
else
    echo "Checking for pre-built binaries..."
    VERSION=$(get_latest_version)

    if [ -n "$VERSION" ]; then
        echo "Latest version: $VERSION"
        echo ""

        if download_binary "$VERSION" "$TARGET"; then
            echo -e "${GREEN}Downloaded pre-built binary successfully!${NC}"
        else
            echo -e "${YELLOW}Pre-built binary not available for $TARGET${NC}"
            echo "Falling back to building from source..."
            echo ""
            build_from_source
        fi
    else
        echo -e "${YELLOW}Could not fetch latest release, falling back to source build...${NC}"
        echo ""
        build_from_source
    fi
fi

echo ""
echo "============================================"
echo -e "${GREEN}Installation complete!${NC}"
echo "============================================"
echo ""
echo "Binary installed to: $INSTALL_DIR/${BINARY_NAME}"
echo ""

# Check if in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo -e "${YELLOW}WARNING: $INSTALL_DIR is not in your PATH${NC}"
    echo ""
    echo "Add this to your shell profile (.bashrc, .zshrc, etc.):"
    echo ""
    echo "  export PATH=\"\$PATH:$INSTALL_DIR\""
    echo ""
fi

echo "Run 'forge --help' to get started."
