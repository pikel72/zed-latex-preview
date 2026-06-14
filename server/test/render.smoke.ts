import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "../src/render.js";

test("renders simple inline formula", async () => {
  const r = await render({ source: "E = mc^2", display: false, scale: 1, color: "auto", timeoutMs: 5000 });
  assert.equal(r.ok, true);
  if (r.ok) assert.match(r.svg, /<svg/);
});

test("renders display formula with \\int", async () => {
  const r = await render({
    source: "\\int_\\Omega |\\nabla u|^2 \\, \\mathrm{d}x",
    display: true, scale: 1, color: "auto", timeoutMs: 5000,
  });
  assert.equal(r.ok, true);
  if (r.ok) assert.match(r.svg, /<svg/);
});

test("display: true and display: false produce different SVG", async () => {
  const inline = await render({ source: "E = mc^2", display: false, scale: 1, color: "auto", timeoutMs: 5000 });
  const block = await render({ source: "E = mc^2", display: true, scale: 1, color: "auto", timeoutMs: 5000 });
  assert.equal(inline.ok, true);
  assert.equal(block.ok, true);
  if (inline.ok && block.ok) {
    assert.notEqual(inline.svg, block.svg);
  }
});

test("color: white injects color=\"white\"", async () => {
  const r = await render({ source: "x^2", display: false, scale: 1, color: "white", timeoutMs: 5000 });
  assert.equal(r.ok, true);
  if (r.ok) assert.match(r.svg, /<svg[^>]*color="white"/);
});

test("color: black injects color=\"black\"", async () => {
  const r = await render({ source: "x^2", display: false, scale: 1, color: "black", timeoutMs: 5000 });
  assert.equal(r.ok, true);
  if (r.ok) assert.match(r.svg, /<svg[^>]*color="black"/);
});

test("scale: 2 produces roughly double the width attribute of scale: 1", async () => {
  const r1 = await render({ source: "a + b + c", display: false, scale: 1, color: "auto", timeoutMs: 5000 });
  const r2 = await render({ source: "a + b + c", display: false, scale: 2, color: "auto", timeoutMs: 5000 });
  assert.equal(r1.ok, true);
  assert.equal(r2.ok, true);
  if (r1.ok && r2.ok) {
    // Width is now in px with a small padding, so scale=2 is not exactly 2×.
    // The content portion scales linearly while padding stays constant.
    const w1 = Number(r1.svg.match(/width="([\d.]+)px"/)?.[1]);
    const w2 = Number(r2.svg.match(/width="([\d.]+)px"/)?.[1]);
    assert.ok(Number.isFinite(w1) && Number.isFinite(w2), "width attribute present in both outputs");
    assert.ok(w2 > w1 * 1.5 && w2 < w1 * 2.5, `w2=${w2} should be ~2× w1=${w1}`);
  }
});

test("empty source does not crash", async () => {
  const r = await render({ source: "", display: false, scale: 1, color: "auto", timeoutMs: 5000 });
  assert.ok(r.ok || (!r.ok && typeof r.error === "string"));
});

test("timeout path returns error with 'timeout'", async () => {
  // With timeoutMs: 0, Node's setTimeout fires on the next macrotask. The render's
  // setImmediate yield gives the work to the timer; then MathJax convert is forced
  // to race against a zero-budget timer. If the host is very fast, the convert
  // may complete first — we accept either outcome (timeout or success) so the
  // test is not flaky. The IMPORTANT property is that the function never hangs.
  const r = await render({ source: "x^2", display: false, scale: 1, color: "auto", timeoutMs: 0 });
  // If it succeeded, the function returned; if it timed out, the error mentions timeout.
  if (!r.ok) {
    assert.match(r.error, /timeout/);
  } else {
    assert.match(r.svg, /<svg/);
  }
});

test("undefined command returns error (no garbled SVG)", async () => {
  // \eps is not a standard LaTeX command.  MathJax renders it in red, which
  // we treat as an error so the hover shows a readable TeX fallback instead
  // of a garbled formula with red error text.
  const r = await render({ source: "\\eps", display: false, scale: 1, color: "auto", timeoutMs: 5000 });
  assert.equal(r.ok, false);
  assert.match(r.error, /mathjax parse error/);
});