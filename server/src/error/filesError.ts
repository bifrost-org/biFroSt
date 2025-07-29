import { StatusCodes } from "http-status-codes";

export class FileError extends Error {
  public statusCode: number;

  constructor(message: string, statusCode: number) {
    super(message);
    this.statusCode = statusCode;
  }

  static NotFound(message = "No such file or directory") {
    return new FileError(message, StatusCodes.NOT_FOUND);
  }

  static InvalidPath(message = "The provided path is invalid or malformed") {
    return new FileError(message, StatusCodes.BAD_REQUEST);
  }
}
