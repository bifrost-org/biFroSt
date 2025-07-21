import { StatusCodes } from "http-status-codes";

export class FileError extends Error {
  public statusCode: number;

  constructor(message: string, statusCode: number) {
    super(message);
    this.statusCode = statusCode;
  }

  static NotFound(message = "File or directory not found") {
    return new FileError(message, StatusCodes.NOT_FOUND);
  }
}
