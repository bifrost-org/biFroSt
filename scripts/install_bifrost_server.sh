#!/usr/bin/env bash
set -euo pipefail

# -----------------------------
# Functions
# -----------------------------
info()  { echo -e "\033[1;34m[INFO]\033[0m $*"; }
error() { echo -e "\033[1;31m[ERROR]\033[0m $*"; exit 1; }
prompt() {
  local message="$1"
  local default="$2"
  local input
  read -r -p "$message [$default]: " input
  if [[ -z "$input" ]]; then
    echo "$default"
  else
    echo "$input"
  fi
}

escape_path() {
    local path="$1"
    printf '%q' "$path"
}

# -----------------------------

info "Starting biFrǫSt server installation..."
echo

# -----------------------------
# Preliminary checks
# -----------------------------
if [[ "$(uname)" != "Linux" ]]; then
    error "biFrǫSt server is only supported on Linux."
fi

command -v git >/dev/null 2>&1 || error "Git not found. Please install it first."

command -v systemctl >/dev/null 2>&1 || error "systemd not found. Cannot create a system service. Please install systemd or start the server manually."

# -----------------------------
# Install system dependencies
# -----------------------------
if ! command -v node >/dev/null 2>&1; then
    info "Node.js not found. Installing..."
    if command -v apt-get >/dev/null 2>&1; then
        sudo apt-get install -y nodejs npm
    elif command -v dnf >/dev/null 2>&1 || command -v yum >/dev/null 2>&1; then
        PKG_CMD="$(command -v dnf >/dev/null 2>&1 && echo dnf || echo yum)"
        sudo $PKG_CMD install -y nodejs npm
    else
        error "Unsupported package manager. Please install Node.js and npm manually."
    fi
fi

# -----------------------------
# Clone and build server
# -----------------------------
REPO_URL="https://github.com/bifrost-org/biFroSt"
REPO_DIR="$HOME/biFroSt"
INSTALL_DIR="$HOME/bifrost-server"

info "Cloning repository into $REPO_DIR..."
rm -rf "$REPO_DIR"
git clone --depth 1 "$REPO_URL" "$REPO_DIR"

cd "$REPO_DIR/server"
info "Installing dependencies..."
npm install
npm run build

# -----------------------------
# Setup environment variables
# -----------------------------
info "Bifrost server configuration setup:"
echo "Press ENTER to use the default value (shown in brackets)"
echo

# Prompt values
DB_HOST=$(prompt "PostgreSQL host" "localhost")
DB_PORT=$(prompt "PostgreSQL port" "5432")
DB_NAME=$(prompt "Database name" "bifrost")
DB_USER=$(prompt "Database user" "heimdallr")
DB_PASSWORD=$(prompt "Database password" "")

PORT=$(prompt "Application port" "3000")
USERS_PATH=$(prompt "Users root path" "$HOME/bifrost-mount/")

# Master key handling
while true; do
    MASTER_KEY=$(prompt "Master key (32-byte hex, leave empty to auto-generate)" "")
    if [[ -z "$MASTER_KEY" ]]; then
        MASTER_KEY=$(openssl rand -hex 32)
        echo "MASTER_KEY generated: $MASTER_KEY"
        break
    elif [[ "$MASTER_KEY" =~ ^[0-9a-fA-F]{64}$ ]]; then
        info "Master key accepted."
        break
    else
        echo "  Invalid key. Must be exactly 64 hex characters (32 bytes). Try again."
    fi
done

echo
info "Configuration summary:"
echo "  DB Host: $DB_HOST"
echo "  DB Port: $DB_PORT"
echo "  DB Name: $DB_NAME"
echo "  DB User: $DB_USER"
echo "  DB Password: $DB_PASSWORD"
echo "  Application port: $PORT"
echo "  Users path: $USERS_PATH"
echo "  Master key: $MASTER_KEY"
echo

# Create .env file
cat > dist/.env <<EOF
# PostgreSQL database configuration
DB_HOST=$DB_HOST
DB_PORT=$DB_PORT
DB_NAME=$DB_NAME
DB_USER=$DB_USER
DB_PASSWORD=$DB_PASSWORD

# Port the app listens on
PORT=$PORT

# MASTER_KEY is the symmetric key used by the server to encrypt and decrypt users' secret keys.
# It must be exactly 32 bytes (64 hexadecimal characters) for AES-256-GCM encryption.
# Generate it with: `openssl rand -hex 32`.
# DO NOT change this key after generation, or previously encrypted data will become unrecoverable.
MASTER_KEY=$MASTER_KEY

# Root path containing all users' directories
USERS_PATH=$USERS_PATH
EOF

info ".env file created successfully"

# Ensure USERS_PATH exists
if [[ ! -d "$USERS_PATH" ]]; then
    info "Creating users root path at $USERS_PATH..."
    mkdir -p "$USERS_PATH"
fi

rm -rf "$INSTALL_DIR"
mkdir "$INSTALL_DIR"

cp -r dist/* "$INSTALL_DIR"
cp dist/.env "$INSTALL_DIR"
cp package*.json "$INSTALL_DIR"

rm -rf "$REPO_DIR"

cd "$INSTALL_DIR"
npm install --omit=dev

# -----------------------------
# Create systemd service
# -----------------------------
SERVICE_FILE="/etc/systemd/system/bifrost-server.service"

info "Creating systemd service at $SERVICE_FILE..."

sudo tee "$SERVICE_FILE" > /dev/null <<EOF
[Unit]
Description=biFrǫSt Server
After=network.target

[Service]
ExecStart=$(command -v node) "$INSTALL_DIR/index.js"
WorkingDirectory=$INSTALL_DIR
Restart=on-failure
User=$(whoami)

[Install]
WantedBy=multi-user.target
EOF

# -----------------------------
# Enable and start service
# -----------------------------
info "Enabling and starting bifrost-server..."
sudo systemctl daemon-reload
sudo systemctl enable bifrost-server
sudo systemctl start bifrost-server

info "Done! Server is running as a systemd service (bifrost-server)."