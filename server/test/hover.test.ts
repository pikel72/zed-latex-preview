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
  const r = await hoverFor(src, { line: 0, character: 7 }, cfg, { R: { body: "\\mathbb{R}", arity: 0 } });
  assert.ok(r);
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("document macro overrides preamble macro of same name", async () => {
  const src = "\\newcommand{\\R}{\\mathcal{R}}\n$\\R$";
  // Preamble defines \R = \mathbb{R}, but document overrides with \mathcal{R}.
  // Cursor on \R (line 1, character 1 = the backslash).
  const r = await hoverFor(src, { line: 1, character: 1 }, cfg, { R: { body: "\\mathbb{R}", arity: 0 } });
  assert.ok(r);
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});

// CRLF regression: positionToOffset must skip \r.  On Windows-saved files,
// every line ends with \r\n; counting \r as a character shifts the cursor
// onto the \n byte at column 0 of the next line, so the math region one
// column over is missed.
test("hover on CRLF document at column 0 finds the math region", async () => {
  const src = "preamble\r\n$x$";
  // line 1, column 0 should land on the opening `$` (offset 10), not the
  // preceding \n (offset 9).
  const r = await hoverFor(src, { line: 1, character: 0 }, cfg);
  assert.ok(r, "expected a hover result on a CRLF line");
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("hover on CRLF document after several lines lands at the right byte", async () => {
  // Multi-line CRLF: each \r adds a drift if counted as a column character.
  const src = "line1\r\nline2\r\nline3\r\n$a+b$";
  // line 3, column 0 should be the `$` after the third \r\n.
  const r = await hoverFor(src, { line: 3, character: 0 }, cfg);
  assert.ok(r, "expected a hover result on a CRLF line after multiple CRLFs");
  assert.match(r!.contents.value, /^!\[formula\]\(data:image\/svg\+xml;base64,/);
});
