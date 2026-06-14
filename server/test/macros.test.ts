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
