import { z } from "zod";
import dotenv from "dotenv";

dotenv.config();

const envSchema = z.object({
  DB_HOST: z.string().min(1),
  DB_PORT: z
    .string()
    .regex(/^\d+$/, "Must be a number")
    .transform(Number)
    .default("5432"),
  DB_NAME: z.string().min(1),
  DB_USER: z.string().min(1),
  DB_PASSWORD: z.string().min(1),
  PORT: z
    .string()
    .regex(/^\d+$/, "Must be a number")
    .transform(Number)
    .default("3000"),
  MASTER_KEY: z
    .string()
    .regex(/^[0-9a-f]{64}$/, "Must be composed of 64 hexadecimal characters"),
  USERS_PATH: z.string().regex(/^\/.*/, "Must be an absolute path"),
});

const envParsed = envSchema.safeParse(process.env);

if (!envParsed.success) {
  console.error("Invalid environment variables:");
  const issues = envParsed.error.format();
  for (const key in issues) {
    const value = issues[key as keyof typeof issues];

    if (!value) continue;

    if (Array.isArray(value)) {
      if (value.length > 0) {
        console.log(`  ${key}: ${value.join(", ")}`);
      }
    } else if ("_errors" in value && value._errors.length > 0) {
      console.log(`  ${key}: ${value._errors.join(", ")}`);
    }
  }
  process.exit(1);
}

export const env = envParsed.data;
