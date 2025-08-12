import NodeCache from "node-cache";

class NonceCache {
  private static cache = new NodeCache({ stdTTL: 300, checkperiod: 60 }); // 5 min TTL

  static has(nonce: string): boolean {
    return this.cache.has(nonce);
  }

  static set(nonce: string): void {
    this.cache.set(nonce, true);
  }
}

export default NonceCache;
