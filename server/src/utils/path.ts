import path from "path";

export const USERS_PATH = process.env.USERS_PATH!;

export function getPath(a: string | undefined, b: string): string {
  return path.join(a!, b);
}

export function getUserPath(username: string, apiKey: string): string {
  return getPath(USERS_PATH, `${username}_${apiKey}`);
}
