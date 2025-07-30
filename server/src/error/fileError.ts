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

  static InvalidPath(message = "The provided path is invalid or malformed") {
    return new FileError(message, StatusCodes.BAD_REQUEST);
  }

  static NotFound(message = "No such file or directory") {
    return new FileError(message, StatusCodes.NOT_FOUND);
  }
}
