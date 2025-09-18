# biFrǫSt

**biFrǫSt** is a remote file system that mounts a virtual folder and translates file operations into **HTTP requests** handled by a stateless REST API server.  
It allows applications to interact with remote files as if they were stored locally.

> Currently, biFrǫSt is primarily supported on **Linux** (via FUSE3). Other platforms may work with additional effort but are not officially supported.

## Features

- **Remote mount** via [FUSE3](https://github.com/libfuse/libfuse) (Rust client).
- **REST API** server built with Node.js/Express (TypeScript).
- **HMAC-based authentication** with timestamp and nonce validation (replay-attack protection).
- **File and directory operations**:

  - Read/write/append/truncate files;
  - Create/delete/move/rename files and directories;
  - Support for symbolic and hard links.

- **Metadata management**: size, permissions, timestamps.
- **Range requests** for efficient large file access.
- Client-side **caching** with automatic invalidation.

## Architecture

- **Client (Rust)**

  - Mounts the virtual file system;
  - Provides also commands for registration and configuration;
  - Converts FUSE calls into signed HTTP requests;
  - Implements caching.

- **Server (TypeScript)**

  - Stateless RESTful service;
  - Validates requests with HMAC;
  - Handles large file streaming with Range requests;
  - Handles multiple users.

## Authentication

All requests are signed with **HMAC-SHA256**.
See [Authentication](./API%20documentation.md#authentication) for details.

HMAC provides:

- **Authentication**: verifies that the request comes from a valid user;
- **Integrity**: ensures that requests and responses are not tampered with in transit;
- **Replay protection**: timestamps and nonces prevent reuse of old requests.

> **Confidentiality is not provided by HMAC**. The data is not encrypted in transit.
> To protect sensitive information, you should use **HTTPS** (TLS/SSL) to encrypt communication between client and server.
> For example, you can deploy a reverse proxy like **Nginx** with a valid SSL certificate to secure your biFrǫSt server.

## API Documentation

- Full API reference: [API documentation](./API%20documentation.md)

## Installation

### Client installation

You can install the client by running the provided command:

```bash
wget -qO- https://raw.githubusercontent.com/bifrost-org/biFroSt/main/scripts/install_bifrost_client.sh | bash
```

Alternatively, you can build the client from source:

```bash
git clone https://github.com/bifrost-org/biFroSt.git
cd biFroSt/client
cargo build --release
```

The compiled binary will be in `target/release/bifrost`. You can move it to a folder in your `PATH`, e.g.:

```bash
sudo mv target/release/bifrost /usr/local/bin/
```

### Server installation

You can install the client by running the provided command:

```bash
wget -qO- https://raw.githubusercontent.com/bifrost-org/biFroSt/main/scripts/install_bifrost_server.sh | bash
```

Alternatively, you can build the server from source:

```bash
git clone https://github.com/bifrost-org/biFroSt.git
cd biFroSt/server
npm install        # install dependencies
npm run build      # compile TypeScript into JavaScript
```

The compiled server will be in dist/. You can run it with:

```bash
node dist/server.js
```

Or create a system service / daemon to run it in the background.

## Usage

### Configure the client

```bash
bifrost config
```

Configures the client. You will be prompted for:

- Server address (hostname or IP);
- Server port;
- Mount point (local folder where the virtual file system will be mounted);
- Timeout (in seconds).

The configuration is saved in `~/.bifrost`.

### Register a new user

```bash
bifrost register
```

Creates a new user on the server using your system username.
The server will return an `api_key` and a `secret_key`, which are stored in `~/.bifrost/`.

### Start the client

```bash
bifrost start [OPTIONS]
```

Starts the client for the current user using the previously configured settings.
The virtual file system will be mounted at the mount point defined in `bifrost config`.

Options:

- `-d`, `--detached` → run the client as a background daemon
- `-e`, `--enable-autorun` → autorun the program on startup

### Stop the client

```bash
bifrost stop [OPTIONS]
```

Options:

- `-d`, `--disable-autorun` → disable autorun

Unmounts the virtual folder and stops the client.
