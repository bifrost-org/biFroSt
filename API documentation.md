# CloudMount API Documentation

# Index

1. [Base URL](#base-url)
2. Collections  
   2.1. [Collection `files`](#collection-documents)  
   2.2. [Collection `sessions`](#collection-sessions)  
   2.3. [Collection `users`](#collection-users)

<br/>

# Base URL

The base URL for all API requests is:

`https://oberon-torino.ddns.net:3000`

<br/>

# Collection `files`

A collection representing the remote files accessible through the mounted virtual file system. Each file can be read, overwritten, created or deleted. Files are identified by their full path.

### Supported requests

- [GET `/files/{path}`](#get-filespath) – Retrieve the a file
- [PUT `/files/{path}`](#put-filespath) – Create or modify a file
- [DELETE `/files/{path}`](#delete-filespath) – Delete a file
- [GET /list/{path}]
- [POST /mkdir/{path}]

## GET `/files/{path}`

Retrieve the contents of a file located at the specified path.

### Path parameters

- `path`: The full path of the file to retrieve. This must be **percent-encoded**. For example, `/folder/file.txt` → `/folder%2Ffile.txt`.

### Response body

Returns the file contents as raw binary.

### Success status

- `200 Ok`: File content returned successfully.

### Errors

This API can return the following error codes:

- `400 Bad Request`: The provided path is invalid or malformed.
- `401 Unauthorized`: The user is not authenticated.
- `404 Not Found`: The file does not exist.
- `500 Internal Server Error`: An unexpected error occurred on the server.

## PUT `/files/{path}`

Create or replace the file at the specified path with the given content and metadata.
This operation can also be used to **move the file** to a new location.

> **Note:** Being a `PUT` request, all supported fields must be provided — both content and metadata.

### Path parameters

- `path`: The **current** full path of the file.
  If `newPath` is provided in the request body, the file will be moved to that new path.

### Request body (multipart JSON + binary)

This request uses `multipart/form-data` with:

#### `Metadata` (part 1 — JSON)

Full description of the file’s metadata and optional new path:

| **Field**     | **Description**                                          | **Type**               | **Required** |
| ------------- | -------------------------------------------------------- | ---------------------- | ------------ |
| `newPath`     | If provided, the file will be **moved** to this new path | `string` (URL-encoded) | No           |
| `size`        | Size in bytes of the content                             | `number`               | Yes          |
| `permissions` | File permission string (e.g. `rw-r--r--`)                | `string`               | Yes          |
| `modified`    | Last modified timestamp (ISO 8601)                       | `string`               | Yes          |

#### Why include `size`?

Although the file size can technically be determined from the uploaded binary, specifying `size` is important for **Integrity verification**: the server can confirm that the received file matches the declared size, detecting truncation or corruption.

#### `Content` (part 2 — binary)

Raw file contents (binary or text).

### Example multipart body

**Part 1 – JSON (metadata):**

```json
{
  "modified": "2025-07-17T09:42:00Z",
  "newPath": "/documents/test.txt",
  "permissions": "rw-r--r--",
  "size": 1024
}
```

**Part 2 – Binary content:**
(e.g., `Some new content of the file...`)

### Semantics

- If the file does **not exist** at `path`, it is **created**.
- If it **does exist**, it is **overwritten completely**, both content and metadata.
- If `newPath` is provided, the file is **moved** (renamed or relocated) to that path, replacing any existing file.

---

### Success status

- `201 Created`: File created.
- `204 No Content`: File updated or moved successfully.

### Errors

- `400 Bad Request`: Metadata missing or malformed, or content missing.
- `401 Unauthorized`: User not authenticated.
- `403 Forbidden`: User not allowed to write or move the file.
- `404 Not Found`: Source file not found.
- `409 Conflict`: Cannot overwrite destination path (e.g., locked).
- `500 Internal Server Error`: Unexpected server error.

## DELETE `/files/{path}`

Delete a file or directory at the specified path.

### Path parameters

- `path`: The full path of the file or directory to delete (percent-encoded).

### Success status

- `204 No Content`: File or directory successfully deleted.

### Errors

- `400 Bad Request`: The path is invalid or malformed.
- `401 Unauthorized`: The user is not authenticated.
- `404 Not Found`: The specified file or directory does not exist.
- `500 Internal Server Error`: An unexpected error occurred on the server.

## GET `/list/{path}`

List the contents of a directory at the specified path.

### Path parameters

- `path`: The full path of the directory to list (percent-encoded).

### Response body

Returns a JSON array containing objects for each entry in the directory.
Each object includes:

```json
[
  {
    "isDirectory": false,
    "modified": "2025-07-18T16:00:00Z",
    "name": "test.txt",
    "permissions": "rw-r--r--",
    "size": 1024
  },
  {
    "isDirectory": true,
    "modified": "2025-07-17T10:15:32Z",
    "name": "subfolder",
    "permissions": "rwxr-xr-x",
    "size": 0
  }
]
```

> **Note**: Size is `0` for directories.

### Success status

- `200 OK`: Directory contents returned successfully.

### Errors

- `400 Bad Request`: Path is malformed or path represents a file.
- `401 Unauthorized`: User not authenticated.
- `404 Not Found`: Directory does not exist.
- `500 Internal Server Error`: Unexpected error on the server.

---

## POST `/mkdir/{path}`

Create a new directory at the specified path.

### Path parameters

- `path`: The full path of the new directory to create (percent-encoded).

### Request body

This endpoint does not require a request body.

### Success status

- `201 Created`: Directory created successfully.

### Errors

- `400 Bad Request`: Path is invalid or already exists.
- `401 Unauthorized`: User not authenticated.
- `500 Internal Server Error`: Unexpected server error.

<br>

# Collection `sessions`

Handles user session management.

### Supported requests

<br/>

# Collection `users`

A collection describing the variety of users interacting with the Kiruna eXplorer system.

### Supported requests
