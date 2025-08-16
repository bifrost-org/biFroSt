import { Request, Response, NextFunction } from "express";
import fs from "fs/promises";
import User from "../model/user";
import { createHmac, createHash } from "crypto";
import { AuthError } from "../error/authError";
import { UserError } from "../error/userError";
import NonceCache from "../cache/nonceCache";
import UserCache from "../cache/userCache";
import { getUserPath } from "../utils/path";

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
    // Range header for GET /files/{path}
    const range = req.header("Range");

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

    let user = UserCache.get(apiKey);
    if (!user) {
      user = await User.getUser(apiKey);
      if (!user) return next(UserError.Unauthorized());
      UserCache.set(apiKey, user);
    }

    const method = req.method.toUpperCase();
    const path = req.path;

    const messageParts = [method, path, timestamp, nonce];

    if (range) messageParts.push(range);

    const extrasHashed = [];

    // extra can be filled with metadata and content (PUT /files/{path})
    // Use req.body.originalMetadata instead of req.body.metadata to avoid parsing and preserve exact client-server consistency
    if (req.body.originalMetadata)
      extrasHashed.push(
        createHash("sha256")
          .update(JSON.stringify(req.body.originalMetadata))
          .digest("hex")
      );

    if (req.body.content?.path) {
      console.log("Content: " + (await fs.readFile(req.body.content.path)));
      extrasHashed.push(
        createHash("sha256")
          .update(await fs.readFile(req.body.content.path))
          .digest("hex")
      );
    }

    if (extrasHashed.length > 0) {
      messageParts.push(extrasHashed.join("\n"));
    }

    const message = messageParts.join("\n");

    console.log("Message: " + message);

    const hmac = createHmac("sha256", user.secretKey);
    hmac.update(message);
    const expectedSignature = hmac.digest("hex");

    if (signature !== expectedSignature) {
      return next(AuthError.InvalidSignature());
    }

    req.userPath = getUserPath(user.username, user.apiKey);

    next();
  } catch (e) {
    next(e);
  }
}
