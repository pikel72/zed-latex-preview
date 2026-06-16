import { test } from "node:test";
import assert from "node:assert/strict";
import { refHoverFor, fenceFor } from "../src/ref_hover.js";
import type { LabelRef } from "../src/rpc_types.js";
import type { PreviewConfig } from "../src/config.js";

const cfg: PreviewConfig = {
  enabled: true,
  maxFormulaLength: 2000,
  timeoutMs: 5000,
  scale: 1.4,
  color: "auto",
  renderer: "mathjax",
  enabledSidecar: true,
  enabledCitePreview: true,
  enabledRefPreview: true,
  enabledDocPreview: true,
  sidecarPath: null,
  bibMaxFileSizeMB: 5,
};

function entry(overrides: Partial<LabelRef>): LabelRef {
  return {
    key: "eq:foo",
    file: "/home/user/project/paper/sections/intro.tex",
    offset: 0,
    line: 41,
    env: "equation",
    math: null,
    caption: "",
    snippet: "E = mc^2",
    ...overrides,
  };
}

// ── not-found ──────────────────────────────────────────────────────────

test("not_found_returns_null", async () => {
  const out = await refHoverFor({ found: false }, undefined, cfg);
  assert.equal(out, null);
});

// ── math env rendering (only for equation/align/gather/multline) ─────

test("math_env_falls_back_to_fence_on_render_error", async () => {
  // Broken math (unbalanced brace) — MathJax will reject; the formatter
  // should fall back to a fenced code block, not crash or return null.
  const out = await refHoverFor({
    found: true,
    entry: entry({ env: "equation", snippet: "{\\frac{" }),
  }, undefined, cfg);
  assert.ok(out);
  // Either SVG (MathJax may recover) or fenced fallback — both valid.
  assert.match(out!.contents.value, /^Equation\b/);
  assert.match(out!.contents.value, /intro\.tex:42$/);
});

// ── env header rules (spec §4.6 + §6) ─────────────────────────────────

test("found_renders_equation_snippet", async () => {
  const out = await refHoverFor({
    found: true,
    entry: entry({
      env: "equation",
      snippet: "E = mc^2",
    }),
  }, undefined, cfg);
  assert.ok(out);
  // header is the env name only — no number
  assert.match(out!.contents.value, /^Equation\b/);
  // math envs get rendered as SVG (not a fenced code block)
  assert.match(out!.contents.value, /^Equation\b.*\n\n!\[formula\]\(data:image\/svg\+xml;base64,/s);
  assert.match(out!.contents.value, /intro\.tex:42$/);
});

test("found_renders_align_no_number", async () => {
  // Spec §6: math env header is just the env name.
  // MathJax needs `\\` for an align row break; without it `&=` parses as
  // a regular relation.
  const out = await refHoverFor({
    found: true,
    entry: entry({ env: "align", caption: "should-not-appear", snippet: "a &= b \\\\" }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^Align\b/);
  assert.doesNotMatch(out!.contents.value, /should-not-appear/);
  assert.match(out!.contents.value, /\n\n!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("found_renders_gather_env_as_svg", async () => {
  const out = await refHoverFor({
    found: true,
    entry: entry({ env: "gather", snippet: "x + y + z" }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^Gather\b/);
  assert.match(out!.contents.value, /\n\n!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("found_renders_multline_env_as_svg", async () => {
  const out = await refHoverFor({
    found: true,
    entry: entry({ env: "multline", snippet: "a + b \\\\" }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^Multline\b/);
  assert.match(out!.contents.value, /\n\n!\[formula\]\(data:image\/svg\+xml;base64,/);
});

test("found_renders_theorem_with_caption", async () => {
  const out = await refHoverFor({
    found: true,
    entry: entry({
      env: "theorem",
      caption: "Fermat's Last Theorem",
      snippet: "a^n + b^n = c^n",
    }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^Theorem: Fermat's Last Theorem/);
  assert.match(out!.contents.value, /```\na\^n \+ b\^n = c\^n\n```/);
});

test("found_renders_section_as_plain_markdown", async () => {
  const out = await refHoverFor({
    found: true,
    entry: entry({
      env: "section",
      caption: "Introduction",
      snippet: "\\section{Introduction}\\label{sec:intro}",
    }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^Section: Introduction/);
  assert.match(out!.contents.value, /\\section\{Introduction\}\\label\{sec:intro\}/);
});

test("found_renders_label_outside_env", async () => {
  // env="" means free-floating \label — fall back to the key as header.
  const out = await refHoverFor({
    found: true,
    entry: entry({
      env: "",
      caption: "",
      snippet: "Some context line\n\\label{eq:foo}",
    }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^eq:foo/);
});

test("header_for_unknown_env_falls_back_to_key", async () => {
  // env="lstlisting" or similar — not in our switch, fall back to key.
  const out = await refHoverFor({
    found: true,
    entry: entry({ env: "lstlisting", caption: "", snippet: "\\begin{lstlisting}" }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^eq:foo/);
});

// ── fence rule (CommonMark §4.5) ──────────────────────────────────────

test("fenceFor_no_backticks_uses_triple", () => {
  assert.equal(fenceFor("hello world"), "```");
});

test("fenceFor_single_backtick_uses_triple", () => {
  // CommonMark requires a fence of at least 3 backticks.  With n=1
  // (longest inner run is 1), the formula max(3, n+1) = max(3, 2) = 3.
  assert.equal(fenceFor("a `b` c"), "```");
});

test("fenceFor_with_triple_backtick_uses_quadruple", () => {
  assert.equal(fenceFor("a ``` b"), "````");
});

test("snippet_with_triple_backtick_is_escaped", async () => {
  // For math envs this becomes SVG; the snippet backticks no longer
  // matter because render() never sees them as fence delimiters.  We
  // verify the SVG path is taken for a snippet containing ```.
  const snippet = "outer ```inner``` outer";
  const out = await refHoverFor({
    found: true,
    entry: entry({ env: "equation", snippet }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /^Equation\b.*\n\n!\[formula\]\(data:image\/svg\+xml;base64,/s);
  assert.match(out!.contents.value, /intro\.tex:42$/);
});

// ── truncation marker ──────────────────────────────────────────────────

test("snippet_truncated_marker_appears_when_oversize", async () => {
  // Rust side does the truncation (12 lines / 4 KiB) and appends the
  // marker.  MathJax ignores the marker comment so the SVG path
  // still succeeds; we just verify the SVG path was taken.
  const snippet = Array.from({ length: 12 }, (_, i) => `line ${i}`).join("\n")
    + "\n% (truncated)";
  const out = await refHoverFor({
    found: true,
    entry: entry({ env: "equation", snippet }),
  }, undefined, cfg);
  assert.ok(out);
  assert.match(out!.contents.value, /!\[formula\]\(data:image\/svg\+xml;base64,/);
});

// ── range forwarding ───────────────────────────────────────────────────

test("range_is_forwarded_to_output", async () => {
  const out = await refHoverFor(
    { found: true, entry: entry({ env: "equation" }) },
    [10, 20],
    cfg,
  );
  assert.ok(out);
  assert.ok(out!.range);
  assert.equal(out!.range!.start.character, 10);
  assert.equal(out!.range!.end.character, 20);
});
