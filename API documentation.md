# biFrǫSt API Documentation

# Index

1. [Collection `files`](#collection-documents)
2. [Collection `sessions`](#collection-sessions)
3. [Collection `users`](#collection-users)

<br/>

# Collection `files`

A collection representing the remote files accessible through the mounted virtual file system. Each file can be read, overwritten, created or deleted. Files are identified by their full path.

### Supported requests

- [GET `/files/{path}`](#get-filespath) – Retrieve a file
- [PUT `/files/{path}`](#put-filespath) – Create or update a file
- [DELETE `/files/{path}`](#delete-filespath) – Delete a file
- [GET `/list/{path}`](#get-listpath) - Retrieve the list of files inside a folder
- [POST `/mkdir/{path}`](#post-mkdirpath) - Create a new folder

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
- `401 Unauthorized`: The user is not authenticated. TODO:
- `404 Not Found`: The file does not exist.
- `500 Internal Server Error`: An unexpected error occurred on the server.

## PUT `/files/{path}`

Create or replace the file at the specified path with the given content and metadata. You can provide content, metadata, or both.

- To create an empty file, you may omit the content or send it as empty.
- To update metadata only, send the metadata fields without content.

### Path parameters

- `path`: The **current** full path of the file. FIXME:

### Request body (multipart JSON + binary)

This request uses `multipart/form-data` with:

#### `Metadata` (part 1 - JSON)

Full description of the file’s metadata and optional new path, and writing mode:

| **Field** | **Description**                                                                                                                                                                                                                 | **Type**               | **Required** |
| --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------- | ------------ |
| `newPath` | If provided, the file will be **moved** to this new path                                                                                                                                                                        | `string` (URL-encoded) | No           |
| `size`    | Meaning depends on `mode`:<br>- In `"write"`, `"append"`, and `"write_at"`, it represents the size in bytes of the provided content.<br>- In `"truncate"`, it defines the final size of the file after truncation or expansion. | `number`               | Yes          |
| `atime`   | Last access timestamp (ISO 8601)                                                                                                                                                                                                | `string`               | Yes          |
| `mtime`   | Last content modification timestamp (ISO 8601)                                                                                                                                                                                  | `string`               | Yes          |
| `ctime`   | Last metadata modification timestamp (ISO 8601)                                                                                                                                                                                 | `string`               | Yes          |
| `crtime`  | File creation timestamp (ISO 8601)                                                                                                                                                                                              | `string`               | No           |
| `kind`    | File type: one of "regular_file", "soft_link", "hard_link"                                                                                                                                                                      | `string`               | Yes          |
| `refPath` | Required if `kind` is `"soft_link"` or `"hard_link"`; points to the target file                                                                                                                                                 | `string`               | Conditional  |
| `perm`    | File permission in octal form (e.g. `644`)                                                                                                                                                                                      | `string`               | Yes          |
| `mode`    | Writing mode: one of `"write"`, `"append"`, `"write_at"`, or `"truncate"`                                                                                                                                                       | `string`               | Yes          |
| `offset`  | Offset in bytes at which to start writing (required if `mode` is `"write_at"`)                                                                                                                                                  | `number`               | Conditional  |

#### Why include `size`?

Although the file size can technically be determined from the uploaded binary, specifying `size` is important for **Integrity verification**: the server can confirm that the received file matches the declared size, detecting truncation or corruption.

#### `Content` (part 2 - binary)

Raw file contents (binary).

#### `Field names`

In the multipart/form-data request body, the field names must be:

- `"metadata"` – containing the JSON object with metadata fields;
- `"content"` – containing the raw binary data of the file.

Correctly naming these fields is required for the server to correctly parse and handle the request.

### Example multipart body

**Part 1 – JSON (metadata):**

```json
{
  "size": 1024,
  "atime": "2025-07-30T17:00:00.000Z",
  "mtime": "2025-07-30T17:00:00.000Z",
  "ctime": "2025-07-30T17:00:00.000Z",
  "crtime": "2025-07-30T15:12:30.000Z",
  "kind": "regular_file",
  "perm": "644",
  "mode": "write"
}
```

**Part 2 – Binary content:**
Raw content of the new file.

### Semantics

The behavior of this endpoint depends on the `mode` specified in the metadata:

- **`"write"`**:

  - If the file does **not exist** at the given `path`, it is **created** with the provided content and metadata.
  - If the file **already exists**, its content and metadata are **fully overwritten**.
    <br>If the `content` part is missing, only the metadata is updated.

- **`"append"`**:

  - If the file exists, the provided binary content is **appended** to the end.
  - If the file does not exist, it is **created**, and the content is written normally.
    Metadata is updated accordingly.

- **`"write_at"`**:
  The binary content is written starting at the specified byte `offset`, required in this mode.

  - If the file is shorter than the offset, it is **expanded** with null bytes (`\0`) to reach the offset.
  - Existing bytes from the offset onward are **overwritten**.

- **`"truncate"`**:
  The file is **resized** to the specified `size`. In this mode, the `content` field is ignored.

  - If the current file is longer, it is **truncated**.
  - If shorter, it is **expanded** with null bytes (`\0`).

Additionally:

- If `newPath` is provided, the file is **moved** (renamed or relocated) to that path, replacing any existing file at the destination.
- If `kind` is `"soft_link"` or `"hard_link"`, the field `refPath` must be provided and no content is required.

### Success status

- `201 Created`: File created.
- `204 No Content`: File updated or moved successfully.

### Errors

- `400 Bad Request`:
  - The provided `path` is invalid or malformed;
  - The metadata are missing or malformed;
  - Integrity verification failed: the declared size does not match the actual content length;
  - `kind` is `"soft_link"` or `"hard_link"` but `refPath` is missing
  - Required fields are missing depending on the selected `mode`.
- `401 Unauthorized`: User not authenticated. TODO:
- `404 Not Found`: The file at `path` does not exist and a `newPath` was specified (cannot move non-existent file).
- `409 Conflict`:
  - Parent directory does not exist;
  - File already exists.
- `500 Internal Server Error`: An unexpected error occurred on the server.

## DELETE `/files/{path}`

Delete a file or directory at the specified path.

### Path parameters

- `path`: The full path of the file or directory to delete (percent-encoded).

### Success status

- `204 No Content`: File or directory successfully deleted.

### Errors

- `400 Bad Request`: The provided path is invalid or malformed.
- `401 Unauthorized`: The user is not authenticated. TODO:
- `404 Not Found`: The specified file or directory does not exist.
- `409 Conflict`: The directory at the provided path is not empty.
- `500 Internal Server Error`: An unexpected error occurred on the server.

## GET `/list/{path}`

List the contents of a directory at the specified path.

### Path parameters

- `path`: The full path of the directory to list (percent-encoded).

### Response body

Returns a JSON array of entry objects, each partially following the [metadata schema](#metadata-part-1---json). Some fields may be omitted or not applicable in this context.

In particular:

- The `kind` field will never be `"hard_link"`.
- The fields `newPath`, `mode` and `offset` will never be present in the response.

If the path is a directory, the array contains all its entries; if it's a file, the array contains a single entry.

Example output (fields may vary depending on the entry):

```json
[
  {
    "name": "file.txt",
    "size": 2563,
    "atime": "2025-07-30T09:39:54.099Z",
    "mtime": "2025-07-30T09:39:50.446Z",
    "ctime": "2025-07-30T09:39:50.446Z",
    "crtime": "2025-07-30T09:39:45.796Z",
    "kind": "regular_file",
    "perm": "644",
    "nlink": 1
  },
  {
    "name": "directory",
    "size": 4096,
    "atime": "2025-07-30T16:26:30.682Z",
    "mtime": "2025-07-30T14:41:09.969Z",
    "ctime": "2025-07-30T14:41:12.797Z",
    "crtime": "2025-07-30T14:41:09.969Z",
    "kind": "directory",
    "perm": "755",
    "nlink": 2
  }
]
```

### Success status

- `200 OK`: Entry metadata returned successfully.

### Errors

- `400 Bad Request`: The provided path is invalid or malformed.
- `401 Unauthorized`: User not authenticated. TODO:
- `404 Not Found`: The specified entry does not exist.
- `500 Internal Server Error`: An unexpected error occurred on the server.

## POST `/mkdir/{path}`

Create a new directory at the specified path.

### Path parameters

- `path`: The full path of the new directory to create (percent-encoded).

### Request body

This endpoint does not require a request body.

### Success status

- `201 Created`: Directory created successfully.

### Errors

- `400 Bad Request`: The provided path is invalid or malformed.
- `401 Unauthorized`: User not authenticated. TODO:
- `404 Not Found`: Parent directory does not exist.
- `409 Conflict`: The directory at the provided path already exists.
- `500 Internal Server Error`: An unexpected error occurred on the server.

<br>

# Collection `sessions`

Handles user session management.

### Supported requests

<br/>

# Collection `users`

A collection describing the variety of users interacting with the Kiruna eXplorer system.

### Supported requests
