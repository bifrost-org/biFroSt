import NodeCache from "node-cache";
import User from "../model/user";

class UserCache {
  private static cache = new NodeCache({ stdTTL: 600, checkperiod: 120 }); // 10 min TTL

  static get(apiKey: string): User | undefined {
    return this.cache.get<User>(apiKey);
  }

  static set(apiKey: string, user: User): void {
    this.cache.set(apiKey, user);
  }
}

export default UserCache;
