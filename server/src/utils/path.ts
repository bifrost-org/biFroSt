import path from "path";

export const USER_PATH = path.resolve(process.env.USERS_PATH!);

export function getPath(a: string | undefined, b: string): string {
  return path.join(a!, b);
}
