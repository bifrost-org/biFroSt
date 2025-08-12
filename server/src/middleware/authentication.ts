import { Request, Response, NextFunction } from "express";
import User from "../model/user";
import { createHmac, createHash } from "crypto";
import { AuthError } from "../error/authError";
import { UserError } from "../error/userError";
import NonceCache from "../cache/nonceCache";

export async function checkAuth(
  req: Request,
  res: Response,
  next: NextFunction
) {
  try {
    const apiKey = req.header("X-Api-Key");
    const signature = req.header("X-Signature");
    const timestamp = req.header("X-Timestamp");
    const nonce = req.header("X-Nonce");

    if (!apiKey || !signature || !timestamp || !nonce) {
      return next(AuthError.MissingHeaders());
    }

    // Max 5 minutes of difference to avoid replay attacks
    const now = Date.now();
    const reqTime = parseInt(timestamp, 10);
    if (isNaN(reqTime) || Math.abs(now - reqTime) > 5 * 60 * 1000) {
      return next(AuthError.InvalidTimestamp());
    }

    if (NonceCache.has(nonce)) return next(AuthError.ReplayAttack());
    NonceCache.set(nonce);

    const user = await User.getUser(apiKey);
    if (!user) {
      return next(UserError.Unauthorized());
    }

    const method = req.method.toUpperCase();
    const path = req.path;

    // only PUT /files/{path} has something extra like multipart-form
    // other routes that require authentication don't have query parameters and body
    let extraHashed;
    if (req.body.originalMetatada) {
      extraHashed = createHash("sha256")
        .update(JSON.stringify(req.body.originalMetatada))
        .digest("hex");
    }

    const message = extraHashed
      ? `${method}\n${path}\n${timestamp}\n${nonce}\n${extraHashed}`
      : `${method}\n${path}\n${timestamp}\n${nonce}`;

    const hmac = createHmac("sha256", user.secretKey);
    hmac.update(message);
    const expectedSignature = hmac.digest("hex");

    if (signature !== expectedSignature) {
      return next(AuthError.InvalidSignature());
    }

    next();
  } catch (e) {
    next(e);
  }
}
