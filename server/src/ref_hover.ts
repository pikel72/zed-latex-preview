//! Markdown formatter for `\ref{...}` hover previews.
//!
//! Takes a `LabelRef` (from `rust_sidecar.lookup("...", "ref")`) and
//! produces the markdown block shown in the hover popup, per
//! `docs/superpowers/specs/2026-06-16-phase2-ref-hover-watcher-design.md`
//! §4.6.
//!
//! The output looks like:
//!
//! ```text
//! Theorem: Fermat's Last Theorem
//!
//! ```latex
//! \begin{theorem}[Fermat's Last Theorem]
//!   a^n + b^n = c^n \implies n \le 2
//! \end{theorem}
//! ```
//!
//! paper/sections/intro.tex:42
//! ```
//!
//! No SVG, no MathJax — purely a text snippet plus a file pointer.

import type { LabelRef } from "./rpc_types.js";

// ── types ──────────────────────────────────────────────────────────────

export interface RefHover {
  contents: { kind: "markdown"; value: string };
  range?: { start: { line: number; character: number }; end: { line: number; character: number } };
}

// ── header: rule from spec §4.6 / §3 step 6 ────────────────────────────
//
// * Section labels: "Section: <caption>".
// * Theorem-like envs: "<Env>: <caption>"  (or "<Env>" alone if empty).
// * Math envs: just the env name ("Equation", "Align", "Gather", ...).
//   `LabelEntry` has no numbering field so we don't render a number.
// * Free-floating label or unknown env: fall back to the key.

function titleCase(env: string): string {
  if (!env) return env;
  // Drop a trailing star: "theorem*" -> "theorem", then capitalise.
  const base = env.endsWith("*") ? env.slice(0, -1) : env;
  return base.charAt(0).toUpperCase() + base.slice(1);
}

function headerFor(entry: LabelRef): string {
  const env = entry.env ?? "";
  const caption = entry.caption ?? "";
  if (env === "section") {
    return caption ? `Section: ${caption}` : `Section: ${entry.key}`;
  }
  // Theorem-like envs: theorem, lemma, proposition, corollary, definition,
  // remark, example, claim, conjecture.
  const theoremEnvs = new Set([
    "theorem",
    "theorem*",
    "lemma",
    "lemma*",
    "proposition",
    "proposition*",
    "corollary",
    "corollary*",
    "definition",
    "definition*",
    "remark",
    "remark*",
    "example",
    "example*",
    "claim",
    "claim*",
    "conjecture",
    "conjecture*",
  ]);
  if (theoremEnvs.has(env)) {
    const label = titleCase(env);
    return caption ? `${label}: ${caption}` : label;
  }
  // Math envs (equation, align, gather, multline, …): env name only.
  const mathEnvs = new Set([
    "equation",
    "equation*",
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
  ]);
  if (mathEnvs.has(env)) {
    return titleCase(env);
  }
  // Free-floating label or unknown env: fall back to the key.
  return entry.key;
}

// ── fence: CommonMark §4.5 — N+1 backticks when the body contains a run ─

/**
 * Pick the fence length for a fenced code block containing `snippet`.
 *
 * CommonMark §4.5: a code fence is a sequence of at least three
 * consecutive back-tick (`` ` ``) characters.  An info string may not
 * contain any back-tick characters.  Inside the block, a fence line is
 * one that begins with the same number of back-ticks.  To safely wrap a
 * snippet, the fence length must exceed the length of any back-tick run
 * inside the snippet itself.
 *
 * We scan for the longest run of consecutive `` ` `` characters; let
 * that length be N.  We return a fence of N+1 back-ticks.  With N=0
 * (snippet has no back-ticks) this degenerates to a triple-back-tick
 * fence, the conventional markdown case.
 */
export function fenceFor(snippet: string): string {
  let n = 0;
  let run = 0;
  for (let i = 0; i < snippet.length; i++) {
    if (snippet.charCodeAt(i) === 0x60 /* ` */) {
      run++;
      if (run > n) n = run;
    } else {
      run = 0;
    }
  }
  // CommonMark §4.5: a code fence is a sequence of at least three
  // back-tick characters.  The longest inner run is `n`; we need n+1
  // for the outer fence to be longer than any inner run, with a
  // floor of 3 to satisfy the spec's minimum.
  const fence = "`".repeat(Math.max(3, n + 1));
  return fence;
}

// ── relative path ──────────────────────────────────────────────────────

function relativePath(file: string): string {
  // Normalise separators so the marker works on both POSIX and Windows.
  const norm = file.replace(/\\/g, "/");
  // Trim down to last two segments (".../sections/intro.tex") so the
  // hover popup stays narrow on long absolute paths.
  const parts = norm.split("/").filter((p) => p.length > 0);
  if (parts.length <= 3) return norm;
  return ".../" + parts.slice(-2).join("/");
}

// ── the main formatter ─────────────────────────────────────────────────

export function refHoverFor(
  result: { found: true; entry: LabelRef } | { found: false },
  range?: [number, number],
): RefHover | null {
  if (!result.found) return null;
  const e = result.entry;
  const header = headerFor(e);
  const fence = fenceFor(e.snippet);
  // The fenced block always opens/closes on its own line so the markdown
  // is portable (CommonMark requires fenced-block delimiters on their
  // own line).
  const snippetBlock = `${fence}\n${e.snippet}\n${fence}`;
  const location = `${relativePath(e.file)}:${e.line + 1}`;
  const value = [header, snippetBlock, location].filter(Boolean).join("\n\n");

  const out: RefHover = {
    contents: { kind: "markdown", value },
  };
  if (range) {
    out.range = {
      start: { line: 0, character: range[0] },
      end: { line: 0, character: range[1] },
    };
  }
  return out;
}
