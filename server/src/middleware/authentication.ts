import { Request, Response, NextFunction } from "express";
import User from "../model/user";
import { createHmac, createHash } from "crypto";
import { AuthError } from "../error/authError";
import { UserError } from "../error/userError";

export async function checkAuth(
  req: Request,
  res: Response,
  next: NextFunction
) {
  try {
    // console.log(req.headers);

    const apiKey = req.header("X-Api-Key");
    const signature = req.header("X-Signature");
    const timestamp = req.header("X-Timestamp");

    if (!apiKey || !signature || !timestamp) {
      return next(AuthError.MissingHeaders());
    }

    const user = await User.getUser(apiKey);
    if (!user) {
      return next(UserError.Unauthorized());
    }

    // console.log("\n" + user);

    const method = req.method.toUpperCase();
    const path = req.path;

    let bodyHash = "";
    if (req.body && Object.keys(req.body).length > 0) {
      bodyHash = createHash("sha256")
        .update(JSON.stringify(req.body))
        .digest("hex");
    }

    const message = bodyHash
      ? `${method}\n${path}\n${timestamp}\n${bodyHash}`
      : `${method}\n${path}\n${timestamp}`;

    // console.log("\n" + message);

    const hmac = createHmac("sha256", user.secretKey);
    hmac.update(message);
    const expectedSignature = hmac.digest("hex");

    if (signature !== expectedSignature) {
      return res.status(403).json({ error: "Invalid signature" });
    }

    // console.log("Signature verificata");

    // Potresti anche controllare il timestamp per evitare replay attack (es. max 5 min di differenza)
    /* const now = Date.now();
    const reqTime = parseInt(timestamp, 10);
    if (isNaN(reqTime) || Math.abs(now - reqTime) > 5 * 60 * 1000) {
      return res.status(400).json({ error: "Invalid or expired timestamp" });
    } */

    next();
  } catch (e) {
    next(e);
  }
}
