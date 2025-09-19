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

info "Starting biFrǫSt server installation..."
echo

# -----------------------------
# Preliminary checks
# -----------------------------
if [[ "$(uname)" != "Linux" ]]; then
    error "biFrǫSt server is only supported on Linux."
fi

# -----------------------------
# Install system dependencies
# -----------------------------
command -v git >/dev/null 2>&1 || error "Git not found. Please install it first."

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
INSTALL_DIR="/$HOME/biFroSt"

info "Cloning repository into $INSTALL_DIR..."
rm -rf "$INSTALL_DIR"
git clone --depth 1 "$REPO_URL" "$INSTALL_DIR"

cd "$INSTALL_DIR/server"
info "Installing dependencies..."
npm install #FIXME:
npm run build

# -----------------------------
# Setup environment variables
# -----------------------------
echo "Bifrost server configuration setup:"
echo "Press ENTER to use the default value (shown in brackets)"
echo

# Prompt values
DB_HOST=$(prompt "PostgreSQL host" "localhost")
DB_PORT=$(prompt "PostgreSQL port" "5432")
DB_NAME=$(prompt "Database name" "bifrost")
DB_USER=$(prompt "Database user" "heimdallr")
DB_PASSWORD=$(prompt "Database password" "changeMe123") #FIXME:

PORT=$(prompt "Application port" "3000")
USERS_PATH=$(prompt "Users root path" "/mnt/bifrost/")

# Master key handling
MASTER_KEY=$(prompt "Master key (leave empty to auto-generate)" "") #FIXME:
if [[ -z "$MASTER_KEY" ]]; then
  MASTER_KEY=$(openssl rand -hex 32)
  info "Generated new MASTER_KEY"
fi

echo
info "Configuration summary:"
echo "  DB Host: $DB_HOST"
echo "  DB Port: $DB_PORT"
echo "  DB Name: $DB_NAME"
echo "  DB User: $DB_USER"
echo "  DB Password: ********"
echo "  Application port: $PORT"
echo "  Users path: $USERS_PATH"
echo "  Master key: $MASTER_KEY"
echo

# Create .env file
cat > .env <<EOF
# PostgreSQL database configuration
DB_HOST=$DB_HOST
DB_PORT=$DB_PORT
DB_NAME=$DB_NAME
DB_USER=$DB_USER
DB_PASSWORD=$DB_PASSWORD

# Port the app listens on
PORT=$PORT

# Master key for AES-256-GCM encryption
MASTER_KEY=$MASTER_KEY

# Root path containing all users' directories
USERS_PATH=$USERS_PATH
EOF

info ".env file created successfully"

node $INSTALL_DIR/server/dist/index.js