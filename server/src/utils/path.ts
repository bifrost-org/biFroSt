import path from "path";
import { env } from "../validation/envSchema";

export function getPath(a: string, b: string): string {
  return path.join(a, b);
}

export function getUserPath(username: string, apiKey: string): string {
  return getPath(env.USERS_PATH, `${username}_${apiKey}`);
}
