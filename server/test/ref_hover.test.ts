import { test } from "node:test";
import assert from "node:assert/strict";
import { refHoverFor, fenceFor } from "../src/ref_hover.js";
import type { LabelRef } from "../src/rpc_types.js";

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

test("not_found_returns_null", () => {
  const out = refHoverFor({ found: false });
  assert.equal(out, null);
});

// ── env header rules (spec §4.6 + §6) ─────────────────────────────────

test("found_renders_equation_snippet", () => {
  const out = refHoverFor({
    found: true,
    entry: entry({
      env: "equation",
      snippet: "E = mc^2",
    }),
  });
  assert.ok(out);
  // header is the env name only — no number
  assert.match(out!.contents.value, /^Equation\b/);
  assert.match(out!.contents.value, /```\nE = mc\^2\n```/);
  assert.match(out!.contents.value, /intro\.tex:42$/);
});

test("found_renders_align_no_number", () => {
  // Spec §6: math env header is just the env name.
  const out = refHoverFor({
    found: true,
    entry: entry({ env: "align", caption: "should-not-appear", snippet: "a &= b" }),
  });
  assert.ok(out);
  assert.match(out!.contents.value, /^Align\b/);
  assert.doesNotMatch(out!.contents.value, /should-not-appear/);
});

test("found_renders_theorem_with_caption", () => {
  const out = refHoverFor({
    found: true,
    entry: entry({
      env: "theorem",
      caption: "Fermat's Last Theorem",
      snippet: "a^n + b^n = c^n",
    }),
  });
  assert.ok(out);
  assert.match(out!.contents.value, /^Theorem: Fermat's Last Theorem/);
  assert.match(out!.contents.value, /```\na\^n \+ b\^n = c\^n\n```/);
});

test("found_renders_section_as_plain_markdown", () => {
  const out = refHoverFor({
    found: true,
    entry: entry({
      env: "section",
      caption: "Introduction",
      snippet: "\\section{Introduction}\\label{sec:intro}",
    }),
  });
  assert.ok(out);
  assert.match(out!.contents.value, /^Section: Introduction/);
  assert.match(out!.contents.value, /\\section\{Introduction\}\\label\{sec:intro\}/);
});

test("found_renders_label_outside_env", () => {
  // env="" means free-floating \label — fall back to the key as header.
  const out = refHoverFor({
    found: true,
    entry: entry({
      env: "",
      caption: "",
      snippet: "Some context line\n\\label{eq:foo}",
    }),
  });
  assert.ok(out);
  assert.match(out!.contents.value, /^eq:foo/);
});

test("header_for_unknown_env_falls_back_to_key", () => {
  // env="lstlisting" or similar — not in our switch, fall back to key.
  const out = refHoverFor({
    found: true,
    entry: entry({ env: "lstlisting", caption: "", snippet: "\\begin{lstlisting}" }),
  });
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

test("snippet_with_triple_backtick_is_escaped", () => {
  // The snippet itself contains ```; the wrapper must use 4 backticks.
  const snippet = "outer ```inner``` outer";
  const out = refHoverFor({
    found: true,
    entry: entry({ env: "equation", snippet }),
  });
  assert.ok(out);
  // Should have a 4-backtick fence on both sides (not a 3-backtick one,
  // which would close early at the inner ```).  The fence is followed
  // by `\n` opening and `\n` closing; we just check the closing
  // backticks appear before the file:line pointer.
  assert.match(out!.contents.value, /\n````\n\n\.\.\.\/sections\/intro\.tex:42$/);
});

// ── truncation marker ──────────────────────────────────────────────────

test("snippet_truncated_marker_appears_when_oversize", () => {
  // Rust side does the truncation (12 lines / 4 KiB) and appends the
  // marker; this test verifies the TS formatter preserves the marker
  // in the output rather than stripping it.
  const snippet = Array.from({ length: 12 }, (_, i) => `line ${i}`).join("\n")
    + "\n% (truncated)";
  const out = refHoverFor({
    found: true,
    entry: entry({ env: "equation", snippet }),
  });
  assert.ok(out);
  assert.match(out!.contents.value, /% \(truncated\)/);
});

// ── range forwarding ───────────────────────────────────────────────────

test("range_is_forwarded_to_output", () => {
  const out = refHoverFor(
    { found: true, entry: entry({ env: "equation" }) },
    [10, 20],
  );
  assert.ok(out);
  assert.ok(out!.range);
  assert.equal(out!.range!.start.character, 10);
  assert.equal(out!.range!.end.character, 20);
});
