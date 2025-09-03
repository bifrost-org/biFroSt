import fs from "fs/promises";
import path from "path";
import { constants } from "fs";
import { env } from "../validation/envSchema";

export function getPath(a: string, b: string): string {
  return path.join(a, b);
}

export function getUserPath(username: string, apiKey: string): string {
  return getPath(env.USERS_PATH, `${username}_${apiKey}`);
}

// Check existance and permissions of USERS_PATH
export async function checkUsersPath(): Promise<void> {
  const usersPath = path.resolve(env.USERS_PATH);

  let stats;
  try {
    stats = await fs.stat(usersPath);
  } catch {
    throw Error(
      `The folder specified in 'USERS_PATH' (${usersPath}) does not exist`
    );
  }

  if (!stats.isDirectory()) {
    throw Error(
      `The path specified in 'USERS_PATH' (${usersPath}) exists but is not a directory`
    );
  }

  try {
    await fs.access(
      usersPath,
      constants.R_OK | constants.W_OK | constants.X_OK
    );
  } catch {
    throw Error(
      `Insufficient permissions (read, write, execute) for 'USERS_PATH' (${usersPath})`
    );
  }
}
