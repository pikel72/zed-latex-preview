//! LRU render‑result cache.
//!
//! Caching is keyed by a 7‑tuple combining display mode, scale, colour
//! theme, macro set (both key names and their bodies), and the fully
//! expanded LaTeX source.  The key is length‑prefixed so that adjacent
//! string fields containing the pipe character (`|`) cannot collide.
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

  // ── key serialisation ────────────────────────────────────────────────
  // Length‑prefix each string field so that `source` / `theme` / etc.
  // containing `|` cannot collide with adjacent fields.

  private keyOf(k: CacheKey): string {
    return [
      k.display,
      k.scale,
      k.theme.length, k.theme,
      k.macroBlock.length, k.macroBlock,
      k.source.length, k.source,
    ].join("|");
  }
}
