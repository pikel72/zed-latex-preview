import { test } from "node:test";
import assert from "node:assert/strict";
import { LRU } from "../src/cache.js";

test("LRU evicts oldest", () => {
  const lru = new LRU<string>(2);
  lru.set({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }, "A");
  lru.set({ source: "b", macroBlock: "", theme: "auto", scale: 1, display: false }, "B");
  lru.get({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }); // bumps a
  lru.set({ source: "c", macroBlock: "", theme: "auto", scale: 1, display: false }, "C"); // evicts b
  assert.equal(lru.get({ source: "b", macroBlock: "", theme: "auto", scale: 1, display: false }), undefined);
  assert.equal(lru.get({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }), "A");
  assert.equal(lru.get({ source: "c", macroBlock: "", theme: "auto", scale: 1, display: false }), "C");
});

test("LRU get on miss returns undefined", () => {
  const lru = new LRU<string>(2);
  assert.equal(lru.get({ source: "x", macroBlock: "", theme: "auto", scale: 1, display: false }), undefined);
});

test("LRU set then get round-trip", () => {
  const lru = new LRU<string>(1);
  lru.set({ source: "k", macroBlock: "", theme: "auto", scale: 1, display: false }, "V");
  assert.equal(lru.get({ source: "k", macroBlock: "", theme: "auto", scale: 1, display: false }), "V");
});

test("LRU caches falsy values without dropping them", () => {
  const lru = new LRU<unknown>(4);
  lru.set({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }, null);
  lru.set({ source: "b", macroBlock: "", theme: "auto", scale: 1, display: false }, 0);
  lru.set({ source: "c", macroBlock: "", theme: "auto", scale: 1, display: false }, false);
  lru.set({ source: "d", macroBlock: "", theme: "auto", scale: 1, display: false }, "");
  assert.equal(lru.get({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }), null);
  assert.equal(lru.get({ source: "b", macroBlock: "", theme: "auto", scale: 1, display: false }), 0);
  assert.equal(lru.get({ source: "c", macroBlock: "", theme: "auto", scale: 1, display: false }), false);
  assert.equal(lru.get({ source: "d", macroBlock: "", theme: "auto", scale: 1, display: false }), "");
});

test("LRU keyOf distinct on pipe-bearing source vs theme", () => {
  // JSON-stringified keys are unambiguous: a `|` in one field cannot leak
  // into another.
  const lru = new LRU<string>(4);
  lru.set({ source: "x|y", macroBlock: "ab", theme: "t", scale: 1, display: false }, "FROM_SOURCE");
  lru.set({ source: "x", macroBlock: "ab", theme: "t|y", scale: 1, display: false }, "FROM_THEME");
  assert.equal(lru.get({ source: "x|y", macroBlock: "ab", theme: "t", scale: 1, display: false }), "FROM_SOURCE");
  assert.equal(lru.get({ source: "x", macroBlock: "ab", theme: "t|y", scale: 1, display: false }), "FROM_THEME");
});

test("LRU keyOf distinct on macroBlocks of equal length but different content", () => {
  const lru = new LRU<string>(2);
  lru.set({ source: "x", macroBlock: "\\R", theme: "auto", scale: 1, display: false }, "REALS");
  lru.set({ source: "x", macroBlock: "\\B", theme: "auto", scale: 1, display: false }, "BLUE");
  assert.equal(lru.get({ source: "x", macroBlock: "\\R", theme: "auto", scale: 1, display: false }), "REALS");
  assert.equal(lru.get({ source: "x", macroBlock: "\\B", theme: "auto", scale: 1, display: false }), "BLUE");
});

test("LRU capacity <= 0 is a no-op", () => {
  const lru = new LRU<string>(0);
  lru.set({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }, "A");
  assert.equal(lru.get({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }), undefined);
});

test("LRU update of existing key does not grow capacity", () => {
  const lru = new LRU<string>(1);
  lru.set({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }, "A1");
  lru.set({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }, "A2");
  assert.equal(lru.get({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }), "A2");
  lru.set({ source: "b", macroBlock: "", theme: "auto", scale: 1, display: false }, "B"); // evicts a
  assert.equal(lru.get({ source: "a", macroBlock: "", theme: "auto", scale: 1, display: false }), undefined);
});
