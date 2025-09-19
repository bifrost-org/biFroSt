#!/usr/bin/env bash
set -euo pipefail

# -----------------------------
# Functions
# -----------------------------
info() { echo -e "\033[1;34m[INFO]\033[0m $*"; }
error() { echo -e "\033[1;31m[ERROR]\033[0m $*"; exit 1; }

info "Starting biFrǫSt client installation..."
echo

# -----------------------------
# Preliminary checks
# -----------------------------
if [[ "$(uname)" != "Linux" ]]; then
    error "biFrǫSt client is only supported on Linux."
fi

command -v git >/dev/null 2>&1 || error "Git not found. Please install it first."

if ! command -v cargo >/dev/null 2>&1; then
    info "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

# -----------------------------
# Install system dependencies
# -----------------------------
info "Installing system dependencies..."

if command -v apt-get >/dev/null 2>&1; then
    info "Detected apt-based Linux (Debian/Ubuntu)..."
    sudo apt-get install -y build-essential pkg-config libfuse3-dev
elif command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1; then
    PKG_CMD="$(command -v dnf >/dev/null 2>&1 && echo dnf || echo yum)"
    info "Detected RPM-based Linux ($PKG_CMD)..."
    sudo $PKG_CMD install -y gcc gcc-c++ make pkgconfig fuse3-devel
else
    error "Unknown Linux package manager. Please install build tools and FUSE3 development headers manually."
fi

# -----------------------------
# Clone and build client
# -----------------------------
REPO_URL="https://github.com/bifrost-org/biFroSt"
INSTALL_DIR="$HOME/biFroSt"

info "Cloning repository into $INSTALL_DIR..."
rm -rf "$INSTALL_DIR"
git clone --depth 1 "$REPO_URL" "$INSTALL_DIR"

cd "$INSTALL_DIR/client"
info "Building bifrost..."
cargo build --release

# -----------------------------
# Install binary
# -----------------------------
BIN_NAME="bifrost"
TARGET_BIN="target/release/$BIN_NAME"

if [[ -f "$TARGET_BIN" ]]; then
    info "Installing binary to /usr/local/bin..."
    sudo cp "$TARGET_BIN" /usr/local/bin/
    sudo chmod +x /usr/local/bin/$BIN_NAME
    info "Installation completed! You can run 'bifrost ...' from anywhere."
else
    error "Build failed: $TARGET_BIN not found"
fi

# -----------------------------
# Cleanup
# -----------------------------
info "Cleaning up source repository..."
rm -rf "$INSTALL_DIR"
info "Done."