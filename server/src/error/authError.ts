import { StatusCodes } from "http-status-codes";

export class AuthError extends Error {
  public statusCode: number;

  constructor(message: string, statusCode: number) {
    super(message);
    this.statusCode = statusCode;
  }

  static MissingHeaders(
    message = "Some authentication headers may be missing"
  ) {
    return new AuthError(message, StatusCodes.BAD_REQUEST);
  }

  static InvalidSignature(
    message = "Invalid signature. Authentication failed"
  ) {
    return new AuthError(message, StatusCodes.FORBIDDEN);
  }
}
