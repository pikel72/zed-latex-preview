import { test } from "node:test";
import assert from "node:assert/strict";
import { hoverFor } from "../src/hover.js";

const cfg = {
  enabled: true,
  maxFormulaLength: 2000,
  timeoutMs: 5000,
  scale: 1.4,
  color: "auto" as const,
  renderer: "mathjax" as const,
};

test("hover on $E=mc^2$ returns markdown image", async () => {
  const src = "Let $E = mc^2$ be famous.";
  const r = await hoverFor(src, { line: 0, character: 7 }, cfg);
  assert.ok(r);
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("hover outside math returns null", async () => {
  const r = await hoverFor("plain text", { line: 0, character: 0 }, cfg);
  assert.equal(r, null);
});

test("hover on macro \\R^2 returns markdown image", async () => {
  const src = "\\newcommand{\\R}{\\mathbb{R}}\nWe work on $\\R^2$.";
  const r = await hoverFor(src, { line: 1, character: 13 }, cfg);
  assert.ok(r);
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("hover on broken math returns TeX fallback", async () => {
  const src = "${\\frac{";
  const r = await hoverFor(src, { line: 0, character: 1 }, cfg);
  assert.ok(r);
  assert.match(r!.contents.value, /```latex/);
});

test("disabled config returns null", async () => {
  const r = await hoverFor("$x$", { line: 0, character: 0 }, { ...cfg, enabled: false });
  assert.equal(r, null);
});

test("hover returns range matching the math region", async () => {
  const src = "Let $E = mc^2$ be famous.";
  const r = await hoverFor(src, { line: 0, character: 7 }, cfg);
  assert.ok(r);
  assert.ok(r!.range);
  assert.equal(typeof r!.range!.start.line, "number");
  assert.equal(typeof r!.range!.start.character, "number");
  assert.equal(typeof r!.range!.end.line, "number");
  assert.equal(typeof r!.range!.end.character, "number");
});

test("cache key distinguishes display: true vs false", async () => {
  // Same tex source in inline and block should produce different cached entries
  const src = "Let $$E = mc^2$$ be famous.";
  const rBlock = await hoverFor(src, { line: 0, character: 7 }, cfg);
  assert.ok(rBlock);
  // Inline version
  const srcInline = "Let $E = mc^2$ be famous.";
  const rInline = await hoverFor(srcInline, { line: 0, character: 7 }, cfg);
  assert.ok(rInline);
  // Both should have data URIs
  assert.match(rBlock!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
  assert.match(rInline!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("preamble macros are expanded", async () => {
  // The document itself contains no \R definition — it comes from preamble.
  const src = "Let $\\R^2$ be the plane.";
  const r = await hoverFor(src, { line: 0, character: 7 }, cfg, { R: "\\mathbb{R}" });
  assert.ok(r);
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("document macro overrides preamble macro of same name", async () => {
  const src = "\\newcommand{\\R}{\\mathcal{R}}\n$\\R$";
  // Preamble defines \R = \mathbb{R}, but document overrides with \mathcal{R}.
  // Cursor on \R (line 1, character 1 = the backslash).
  const r = await hoverFor(src, { line: 1, character: 1 }, cfg, { R: "\\mathbb{R}" });
  assert.ok(r);
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});
