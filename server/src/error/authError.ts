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
    message = "Authentication failed: invalid signature"
  ) {
    return new AuthError(message, StatusCodes.UNAUTHORIZED);
  }

  static InvalidTimestamp(
    message = "Authentication failed: timestamp is invalid or expired"
  ) {
    return new AuthError(message, StatusCodes.UNAUTHORIZED);
  }

  static ReplayAttack(message = "Replay attack detected: nonce already used") {
    return new AuthError(message, StatusCodes.UNAUTHORIZED);
  }
}
