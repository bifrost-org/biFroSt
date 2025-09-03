import { strict as assert } from "assert";
import { Pool, PoolClient, QueryResult } from "pg";
import { env } from "./validation/envSchema";

let pool: Pool | undefined;

export class Database {
  static setup() {
    pool = new Pool({
      host: env.DB_HOST,
      port: env.DB_PORT,
      user: env.DB_USER,
      password: env.DB_PASSWORD,
      database: env.DB_NAME,
    });
  }

  static async getClient(): Promise<PoolClient> {
    if (!pool) Database.setup();
    assert(pool);
    return await pool.connect();
  }

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  static async query(text: string, values?: any[]): Promise<QueryResult<any>> {
    if (!pool) Database.setup();
    assert(pool);
    return await pool.query(text, values);
  }

  static async withTransaction<T>(
    action: (client: PoolClient) => Promise<T>
  ): Promise<T> {
    const client = await Database.getClient();
    try {
      await client.query("BEGIN");
      const result = await action(client);
      await client.query("COMMIT");
      return result;
    } catch (error) {
      await client.query("ROLLBACK");
      throw error;
    } finally {
      client.release();
    }
  }

  static async disconnect(): Promise<void> {
    assert(pool);
    await pool.end();
  }
}
