import { StatusCodes } from "http-status-codes";

export class UserError extends Error {
  public statusCode: number;

  constructor(message: string, statusCode: number) {
    super(message);
    this.statusCode = statusCode;
  }

  static RegistrationFailed(message = "User registration failed") {
    return new UserError(message, StatusCodes.BAD_REQUEST);
  }

  static Unauthorized(message = "User does not exist or invalid API key") {
    return new UserError(message, StatusCodes.UNAUTHORIZED);
  }
}
