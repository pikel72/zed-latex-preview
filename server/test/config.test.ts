import { test } from "node:test";
import assert from "node:assert/strict";
import { configFromInit, defaultConfig } from "../src/config.js";

test("configFromInit: undefined init yields defaults", () => {
  const c = configFromInit(undefined);
  const d = defaultConfig();
  assert.equal(c.enabled, d.enabled);
  assert.equal(c.scale, d.scale);
  assert.equal(c.color, d.color);
  assert.equal(c.timeoutMs, d.timeoutMs);
  assert.equal(c.maxFormulaLength, d.maxFormulaLength);
});

test("configFromInit: reads scale and color from initializationOptions", () => {
  // This is the JSON the Rust side now forwards verbatim from
  // lsp.latex-preview.settings as initializationOptions.
  const c = configFromInit({ scale: 2.5, color: "white", enabled: false });
  assert.equal(c.scale, 2.5);
  assert.equal(c.color, "white");
  assert.equal(c.enabled, false);
});

test("configFromInit: ignores unknown keys, keeps defaults for missing", () => {
  const c = configFromInit({ bogus: 1, scale: 3 });
  assert.equal(c.scale, 3);
  assert.equal(c.color, "auto");
  assert.equal(c.timeoutMs, defaultConfig().timeoutMs);
});

test("configFromInit: ignores garbage", () => {
  const c = configFromInit("garbage" as unknown);
  assert.equal(c.scale, defaultConfig().scale);
});
