import { Database } from "../database";
import {
  decryptSecretKey,
  EncryptedSecretKey,
  encryptSecretKey,
  generateApiKey,
  generateSecretKey,
} from "../utils/crypto";

type UserDbRow = {
  id: number;
  username: string;
  api_key: string;
  crypted_secret_key: string;
  iv: string;
  tag: string;
  created_at: Date;
};

class User {
  username: string;
  apiKey: string;
  secretKey: string;
  createdAt: Date;

  constructor(
    username: string,
    apiKey: string,
    secretKey: string,
    createdAt: Date
  ) {
    this.username = username;
    this.apiKey = apiKey;
    this.secretKey = secretKey;
    this.createdAt = createdAt;
  }

  private static fromDatabaseRow(dbRow: UserDbRow): User {
    const { username, api_key, crypted_secret_key, iv, tag, created_at } =
      dbRow;
    return new User(
      username,
      api_key,
      decryptSecretKey(crypted_secret_key, iv, tag),
      created_at
    );
  }

  static async register(username: string): Promise<User | undefined> {
    const apiKey = generateApiKey();
    const secretKey = generateSecretKey();

    const encryptedSecretKey: EncryptedSecretKey = encryptSecretKey(secretKey);

    await Database.query(
      `INSERT INTO "user" (username, api_key, crypted_secret_key, iv, tag) VALUES ($1, $2, $3, $4, $5)`,
      [
        username,
        apiKey,
        encryptedSecretKey.ciphertext,
        encryptedSecretKey.iv,
        encryptedSecretKey.tag,
      ]
    );

    return await this.getUser(apiKey); // check new user and return it
  }

  static async getUser(apiKey: string): Promise<User | undefined> {
    const result = await Database.query(
      `SELECT * FROM "user" WHERE api_key = $1`,
      [apiKey]
    );

    const userRow = result.rows[0];
    if (!userRow) return undefined;

    const user = User.fromDatabaseRow(userRow);

    return user;
  }
}

export default User;
