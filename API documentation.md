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

- `path`: The **current** full path of the file.
  If `newPath` is provided in the request body, the file will be moved to that new path.

### Request body (multipart JSON + binary)

This request uses `multipart/form-data` with:

#### `Metadata` (part 1 - JSON)

Full description of the file’s metadata and optional new path:

| **Field**    | **Description**                                                                               | **Type**               | **Required** |
| ------------ | --------------------------------------------------------------------------------------------- | ---------------------- | ------------ |
| `newPath`    | If provided, the file will be **moved** to this new path                                      | `string` (URL-encoded) | No           |
| `size`       | Size in bytes of the content                                                                  | `number`               | Yes          |
| `atime`      | Last access timestamp (ISO 8601)                                                              | `string`               | Yes          |
| `mtime`      | Last content modification timestamp (ISO 8601)                                                | `string`               | Yes          |
| `ctime`      | Last metadata modification timestamp (ISO 8601)                                               | `string`               | Yes          |
| `crtime`     | File creation timestamp (ISO 8601)                                                            | `string`               | No           |
| `kind`       | File type: one of "directory", "regular_file", "soft_link", "hard_link"                       | `string`               | Yes          |
| `perm`       | File permission in octal form (e.g. `644`)                                                    | `string`               | Yes          |
| `nlink`      | Number of hard links                                                                          | `number`               | Yes          |
| `appendMode` | If `true` and the `content` part is present, the data will be appended instead of overwritten | `boolean`              | No           |

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
  "nlink": 1
}
```

**Part 2 – Binary content:**
Raw content of the new file.

### Semantics

- If the file does **not exist** at the given `path`, it is **created** with the provided content and metadata.
- If the file already exists:
  - If content is present and appendMode is false or absent, the file is fully overwritten (content and metadata replaced);
  - If content is present and appendMode is true, the content is appended to the existing file, and metadata is updated accordingly;
  - If the content part is missing, only the metadata is updated.
- If `newPath` is provided, the file is **moved** (renamed or relocated) to that path, replacing any existing file.

### Success status

- `201 Created`: File created.
- `204 No Content`: File updated or moved successfully.

### Errors

- `400 Bad Request`: The provided path is invalid or malformed, or metadata missing or malformed.
- `401 Unauthorized`: User not authenticated. TODO:
- `404 Not Found`: Source file not found.
- `409 Conflict`: Cannot overwrite destination path (e.g. locked).
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
- The fields `newPath` and `appendMode` will never be present in the response.

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
