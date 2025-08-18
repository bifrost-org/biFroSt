import { StatusCodes } from "http-status-codes";

export class FileError extends Error {
  public statusCode: number;

  constructor(message: string, statusCode: number) {
    super(message);
    this.statusCode = statusCode;
  }

  static DirectoryNotEmpty(
    message = "The directory at the provided path is not empty"
  ) {
    return new FileError(message, StatusCodes.CONFLICT);
  }

  static DirectoryAlreadyExists(
    message = "The directory at the provided path already exists"
  ) {
    return new FileError(message, StatusCodes.CONFLICT);
  }

  static FileAlreadyExists(
    message = "The file at the provided path already exists"
  ) {
    return new FileError(message, StatusCodes.CONFLICT);
  }

  static InvalidPath(message = "The provided path is invalid or malformed") {
    return new FileError(message, StatusCodes.BAD_REQUEST);
  }

  static NotADirectory(message = "Expected a directory, but found a file") {
    return new FileError(message, StatusCodes.BAD_REQUEST);
  }

  static NotFound(message = "The specified file or directory does not exist") {
    return new FileError(message, StatusCodes.NOT_FOUND);
  }

  static OperationNotPermitted(message = "") {
    return new FileError(message, StatusCodes.FORBIDDEN);
  }

  static ParentDirectoryNotFound(message = "Parent directory does not exist") {
    return new FileError(message, StatusCodes.CONFLICT);
  }

  static RequestedRangeNotSatisfiable(
    message = "The specified range is invalid or outside the file size"
  ) {
    return new FileError(message, StatusCodes.REQUESTED_RANGE_NOT_SATISFIABLE);
  }

  static SizeMismatch(
    message = "Size mismatch: integrity verification failed"
  ) {
    return new FileError(message, StatusCodes.BAD_REQUEST);
  }
}
