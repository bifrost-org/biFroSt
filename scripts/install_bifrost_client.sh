#!/usr/bin/env bash
set -euo pipefail

# Functions for messages
info() { echo -e "\033[1;34m[INFO]\033[0m $*"; }
error() { echo -e "\033[1;31m[ERROR]\033[0m $*"; exit 1; }

# Check prerequisites
command -v git >/dev/null 2>&1 || error "Git not found. Please install it first."
command -v cargo >/dev/null 2>&1 || {
    info "Rust not found. Installing..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
}

# Install system dependencies
info "Installing system dependencies..."

OS="$(uname)"
if [[ "$OS" == "Linux" ]]; then
  # Debian/Ubuntu check
  if command -v apt-get >/dev/null 2>&1; then
    info "Detected apt-based Linux (Debian/Ubuntu). Checking packages..."
    PKGS="build-essential pkg-config"
    # check for libfuse3-dev
    if dpkg -s libfuse3-dev >/dev/null 2>&1; then
      info "libfuse3-dev already installed."
    else
      # try to install libfuse3-dev; if not available, fallback message
      sudo apt-get update
      if apt-get install -y libfuse3-dev; then
        info "libfuse3-dev installed."
      else
        error "libfuse3-dev not available in apt repos. You may need a newer distro or install libfuse from source."
      fi
    fi
    # ensure compilers and pkg-config
    sudo apt-get install -y $PKGS || error "Failed to install build-essential/pkg-config."

  # RHEL/Fedora check
  elif command -v yum >/dev/null 2>&1 || command -v dnf >/dev/null 2>&1; then
    PKG_CMD="$(command -v dnf >/dev/null 2>&1 && echo dnf || echo yum)"
    info "Detected RPM-based Linux ($PKG_CMD)."
    # check rpm package
    if rpm -q fuse3-devel >/dev/null 2>&1; then
      info "fuse3-devel already installed."
    else
      sudo $PKG_CMD install -y gcc gcc-c++ make pkgconfig fuse3-devel || error "Failed to install fuse3-devel (or equivalent)."
    fi
  else
    error "Unknown Linux package manager. Please install: build-essential (or gcc/make), pkg-config, and FUSE development headers."
  fi

# macOS branch
elif [[ "$OS" == "Darwin" ]]; then
  info "Detected macOS."
  command -v brew >/dev/null 2>&1 || error "Homebrew not found. Please install it from https://brew.sh/."
  # Prefer libfuse formula for builds, but macfuse cask may be required for runtime/kernel integration.
  if brew list --formula | grep -q '^libfuse$' || brew list --formula | grep -q '^libfuse@3$'; then
    info "libfuse (fuse3) formula already installed."
  else
    info "Installing libfuse (fuse3) formula..."
    brew install libfuse || info "libfuse formula failed or not present; you may need to install macFUSE cask instead."
  fi
  # macFUSE cask
  if brew list --cask | grep -q '^macfuse$'; then
    info "macFUSE cask already installed."
  else
    info "Installing macFUSE (for full macOS FUSE support)..."
    brew install --cask macfuse || error "macFUSE install failed; user may need to approve kernel extension in System Settings -> Security & Privacy."
  fi
  # tools
  brew install pkg-config || error "Failed to install pkg-config via Homebrew."

else
  error "Unsupported OS: $OS. Please install build-essential/pk-config and FUSE dev headers manually."
fi

# Clone the repository
REPO_URL="https://github.com/bifrost-org/biFroSt"
INSTALL_DIR="$HOME/biFroSt"

info "Cloning repository into $INSTALL_DIR..."
rm -rf "$INSTALL_DIR"
git clone --depth 1 "$REPO_URL" "$INSTALL_DIR"

# Build the project
cd "$INSTALL_DIR/client"
info "Building bifrost..."
cargo build --release

# Copy binary to /usr/local/bin (requires sudo)
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

info "Cleaning up source repository..."
rm -rf "$INSTALL_DIR"