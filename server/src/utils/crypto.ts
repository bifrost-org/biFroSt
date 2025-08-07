import crypto from "crypto";

const ALGORITHM = "aes-256-gcm";

if (!process.env.MASTER_KEY) {
  throw new Error("MASTER_KEY not set in .env");
}
const MASTER_KEY = Buffer.from(process.env.MASTER_KEY, "hex"); // 32 byte

export function generateApiKey(): string {
  return crypto.randomBytes(16).toString("hex");
}

export function generateSecretKey(): string {
  return crypto.randomBytes(32).toString("hex");
}

export type EncryptedSecretKey = {
  ciphertext: string;
  iv: string;
  tag: string;
};

export function encryptSecretKey(secretKey: string): EncryptedSecretKey {
  const iv: Buffer = crypto.randomBytes(12); // https://crypto.stackexchange.com/questions/41601/aes-gcm-recommended-iv-size-why-12-bytes
  const cipher = crypto.createCipheriv(ALGORITHM, MASTER_KEY, iv);

  const encrypted = Buffer.concat([
    cipher.update(Buffer.from(secretKey, "hex")),
    cipher.final(),
  ]);

  const tag = cipher.getAuthTag();

  return {
    ciphertext: encrypted.toString("base64"),
    iv: iv.toString("base64"),
    tag: tag.toString("base64"),
  } satisfies EncryptedSecretKey;
}

export function decryptSecretKey(
  ciphertext: string,
  iv: string,
  tag: string
): string {
  const decipher = crypto.createDecipheriv(
    ALGORITHM,
    MASTER_KEY,
    Buffer.from(iv, "base64")
  );
  decipher.setAuthTag(Buffer.from(tag, "base64"));

  const decrypted = Buffer.concat([
    decipher.update(Buffer.from(ciphertext, "base64")),
    decipher.final(),
  ]);

  return decrypted.toString("hex");
}
