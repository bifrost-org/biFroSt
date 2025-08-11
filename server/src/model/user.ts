import { Database } from "../database";
import {
  decryptSecretKey,
  EncryptedSecretKey,
  encryptSecretKey,
  generateApiKey,
  generateSecretKey,
} from "../utils/crypto";
import { getPath, USER_PATH } from "../utils/path";

type UserDbRow = {
  id: number;
  username: string;
  api_key: string;
  crypted_secret_key: string;
  iv: string;
  tag: string;
  created_at: Date;
  home_path: string;
};

class User {
  username: string;
  apiKey: string;
  secretKey: string;
  createdAt: Date;
  homePath: string;

  constructor(
    username: string,
    apiKey: string,
    secretKey: string,
    createdAt: Date,
    homePath: string
  ) {
    this.username = username;
    this.apiKey = apiKey;
    this.secretKey = secretKey;
    this.createdAt = createdAt;
    this.homePath = homePath;
  }

  private static userCache: Map<string, User> = new Map();

  private static fromDatabaseRow(dbRow: UserDbRow): User {
    const {
      username,
      api_key,
      crypted_secret_key,
      iv,
      tag,
      created_at,
      home_path,
    } = dbRow;
    return new User(
      username,
      api_key,
      decryptSecretKey(crypted_secret_key, iv, tag),
      created_at,
      home_path
    );
  }

  static async register(username: string): Promise<User | undefined> {
    const apiKey = generateApiKey();
    const secretKey = generateSecretKey();

    const encryptedSecretKey: EncryptedSecretKey = encryptSecretKey(secretKey);

    await Database.query(
      `INSERT INTO "user" (username, api_key, crypted_secret_key, iv, tag, home_path) VALUES ($1, $2, $3, $4, $5, $6)`,
      [
        username,
        apiKey,
        encryptedSecretKey.ciphertext,
        encryptedSecretKey.iv,
        encryptedSecretKey.tag,
        getPath(USER_PATH, username),
      ]
    );

    return await this.getUser(apiKey); // check new user and return it
  }

  // cached query response
  static async getUser(apiKey: string): Promise<User | undefined> {
    if (this.userCache.has(apiKey)) return this.userCache.get(apiKey);

    const result = await Database.query(
      `SELECT * FROM "user" WHERE api_key = $1`,
      [apiKey]
    );

    const userRow = result.rows[0];
    if (!userRow) return undefined;

    const user = User.fromDatabaseRow(userRow);

    this.userCache.set(apiKey, user);

    return user;
  }
}

export default User;
