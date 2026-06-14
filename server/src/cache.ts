//! LRU render‑result cache.
//!
//! Caching is keyed by a tuple of display mode, scale, colour theme, the
//! merged macro set, and the fully expanded LaTeX source.  The key is
//! JSON‑serialised, so any field containing the same characters as another
//! cannot collide.
//!
//! ## Capacity
//!
//! Default capacity is 256 entries — enough to cover every formula visible
//! in a typical editing session without unbounded memory growth.

export interface CacheKey {
  source: string;      // fully expanded LaTeX source
  macroBlock: string;  // JSON serialisation of the merged macro map
  theme: string;       // "auto" | "black" | "white"
  scale: number;
  display: boolean;
}

export class LRU<V> {
  private map = new Map<string, V>();

  constructor(private capacity: number) {}

  /** Look up a cache entry.  On hit the entry is bumped to the front. */
  get(key: CacheKey): V | undefined {
    const k = this.keyOf(key);
    const v = this.map.get(k);
    if (v !== undefined) {
      this.map.delete(k);
      this.map.set(k, v);  // bump to most‑recently‑used end
    }
    return v;
  }

  /** Insert or update an entry.  Evicts the least‑recently‑used entry when
   *  the capacity is exceeded.  No‑op when capacity ≤ 0. */
  set(key: CacheKey, v: V): void {
    if (this.capacity <= 0) return;
    const k = this.keyOf(key);
    if (this.map.has(k)) this.map.delete(k);
    this.map.set(k, v);
    if (this.map.size > this.capacity) {
      const first = this.map.keys().next().value as string | undefined;
      if (first !== undefined) this.map.delete(first);
    }
  }

  // `JSON.stringify` of a fixed-shape object is unambiguous, so no field
  // can bleed into another regardless of its contents.
  private keyOf(k: CacheKey): string {
    return JSON.stringify([k.display, k.scale, k.theme, k.macroBlock, k.source]);
  }
}
