import { test } from "node:test";
import assert from "node:assert/strict";
import { extractMacros, expand, mergeMacros } from "../src/macros.js";

test("newcommand without args", () => {
  const doc = "\\newcommand{\\R}{\\mathbb{R}}\n\\R^2";
  const m = extractMacros(doc);
  assert.equal(m.R, "\\mathbb{R}");
  assert.equal(expand("\\R^2", m), "\\mathbb{R}^2");
});

test("newcommand with one arg", () => {
  const doc = "\\newcommand{\\norm}[1]{\\left\\lVert #1 \\right\\rVert}";
  const m = extractMacros(doc);
  assert.equal(expand("\\norm{x}", m), "\\left\\lVert x \\right\\rVert");
});

test("newcommand with two args", () => {
  const doc = "\\newcommand{\\foo}[2]{#1+#2}";
  const m = extractMacros(doc);
  assert.equal(expand("\\foo{a}{b}", m), "a+b");
});

test("newcommand with three args (bare-token args)", () => {
  const doc = "\\newcommand{\\bar}[3]{#1,#2,#3}";
  const m = extractMacros(doc);
  assert.equal(expand("\\bar{x}{y}{z}", m), "x,y,z");
});

test("newcommand with mixed brace/bare args", () => {
  const doc = "\\newcommand{\\baz}[2]{[#1|#2]}";
  const m = extractMacros(doc);
  assert.equal(expand("\\baz{a}b", m), "[a|b]");
});

test("newcommand* (starred) is recognized", () => {
  const doc = "\\newcommand*{\\R}{\\mathbb{R}}\n\\R^2";
  const m = extractMacros(doc);
  assert.equal(m.R, "\\mathbb{R}");
  assert.equal(expand("\\R^2", m), "\\mathbb{R}^2");
});

test("renewcommand is recognized", () => {
  const doc = "\\renewcommand{\\R}{\\mathbb{R}}\n\\R^2";
  const m = extractMacros(doc);
  assert.equal(m.R, "\\mathbb{R}");
});

test("\\def is recognized", () => {
  const doc = "\\def\\R{\\mathbb{R}}\n\\R^2";
  const m = extractMacros(doc);
  assert.equal(m.R, "\\mathbb{R}");
  assert.equal(expand("\\R^2", m), "\\mathbb{R}^2");
});

test("DeclareMathOperator", () => {
  const doc = "\\DeclareMathOperator{\\div}{div}\n\\div";
  const m = extractMacros(doc);
  assert.equal(expand("\\div", m), "\\operatorname{div}");
});

test("ignored when malformed", () => {
  const doc = "\\newcommand{\\broken"; // missing body
  const m = extractMacros(doc);
  assert.deepEqual(m, {});
});

test("mergeMacros: overrides win over base", () => {
  const base = { R: "\\mathbb{R}", Z: "\\mathbb{Z}" };
  const overrides = { R: "\\mathcal{R}" };
  const merged = mergeMacros(base, overrides);
  assert.equal(merged.R, "\\mathcal{R}");  // override
  assert.equal(merged.Z, "\\mathbb{Z}");   // from base
});

// Regression: macro bodies and arguments can contain nested braces
// (e.g. \sqrt inside a \newcommand body, or a \frac inside a macro
// argument).  The previous regex-based parser only understood one level
// of `{…}` and silently dropped such definitions.

test("newcommand with one level of nested braces in body", () => {
  const doc = "\\newcommand{\\x}{\\sqrt{\\frac{a}{b}}}";
  const m = extractMacros(doc);
  assert.equal(m.x, "\\sqrt{\\frac{a}{b}}");
  assert.equal(expand("\\x", m), "\\sqrt{\\frac{a}{b}}");
});

test("newcommand with deeply nested braces in body", () => {
  const doc = "\\newcommand{\\y}{\\frac{\\sqrt{a}}{\\sqrt{b}}}";
  const m = extractMacros(doc);
  assert.equal(m.y, "\\frac{\\sqrt{a}}{\\sqrt{b}}");
});

test("newcommand body with two-level nesting in arg of operator", () => {
  // \operatorname-style bodies also nest.
  const doc = "\\DeclareMathOperator{\\BigO}{\\mathcal{O}_{n}}";
  const m = extractMacros(doc);
  assert.equal(m.BigO, "\\operatorname{\\mathcal{O}_{n}}");
});

test("macro argument with nested braces", () => {
  const doc = "\\newcommand{\\foo}[1]{#1+1}";
  const m = extractMacros(doc);
  // The argument itself has nested braces (\frac inside).
  assert.equal(expand("\\foo{\\frac{a}{b}}", m), "\\frac{a}{b}+1");
});

test("macro with nested braces survives roundtrip from extract+expand", () => {
  // Document defines \x with nested body and uses it — the hover expansion
  // must produce the original body verbatim.
  const doc = "\\newcommand{\\x}{\\sqrt{\\frac{a}{b}}}\n$\\x$";
  const m = extractMacros(doc);
  assert.equal(m.x, "\\sqrt{\\frac{a}{b}}");
  assert.equal(expand("\\x", m), "\\sqrt{\\frac{a}{b}}");
});
