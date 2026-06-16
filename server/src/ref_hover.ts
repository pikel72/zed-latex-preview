//! Markdown formatter for `\ref{...}` hover previews.
//!
//! Takes a `LabelRef` (from `rust_sidecar.lookup("...", "ref")`) and
//! produces the markdown block shown in the hover popup, per
//! `docs/superpowers/specs/2026-06-16-phase2-ref-hover-watcher-design.md`
//! §4.6.
//!
//! Math environments (`equation`, `align`, `gather`, `multline`, plus
//! their starred forms) are rendered with MathJax and embedded as a
//! data-URI image — that's the only way the user can actually see the
//! formula they're referencing.  All other envs (theorem, lemma,
//! section, …) get the original fenced-code-block treatment: a text
//! snippet plus a file pointer.

import type { LabelRef } from "./rpc_types.js";
import type { PreviewConfig } from "./config.js";
import { render } from "./render.js";

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

// ── math-env detection ─────────────────────────────────────────────────
//
// Mirrors the set in `refHoverFor` below.  Kept as a small helper so the
// SVG/fence decision is easy to spot at the call site.

const MATH_ENVS = new Set([
  "equation", "equation*",
  "align", "align*",
  "gather", "gather*",
  "multline", "multline*",
]);

/** True when the env's body needs to be fed to MathJax as inline display
 *  math (vs the equation family which is already self-contained). */
function isMathEnv(env: string): boolean {
  return MATH_ENVS.has(env);
}

/** Wrap an env body so MathJax treats it as the corresponding amsmath
 *  display-math construct.  The Rust side strips `\begin{<env>}…\end{<env>}`
 *  from the snippet, leaving only the body.  For equation-like envs the
 *  body is already valid display math; for align/gather/multline the
 *  body uses `&=` / `\\` which MathJax's `ams` package only recognises
 *  inside the corresponding `aligned`/`gathered` wrapper. */
function wrapForRender(env: string, body: string): string {
  switch (env) {
    case "equation":
    case "equation*":
      return body;
    case "gather":
    case "gather*":
      return `\\begin{gathered}\n${body}\n\\end{gathered}`;
    case "align":
    case "align*":
    case "multline":
    case "multline*":
    default:
      return `\\begin{aligned}\n${body}\n\\end{aligned}`;
  }
}

function toDataUri(svg: string): string {
  return "data:image/svg+xml;base64," + Buffer.from(svg, "utf8").toString("base64");
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

/**
 * Format a `\ref{...}` hover preview.
 *
 * - `result.found === false` → returns `null` (caller falls through).
 * - Math envs (equation/align/gather/multline) → rendered to SVG and
 *   embedded as a data URI.  If MathJax rejects the snippet (e.g. an
 *   unbalanced `\frac`), we fall back to the original fenced block
 *   rather than dropping the hover — a partial preview beats nothing.
 * - Everything else → fenced code block + header + file:line pointer.
 *
 * The signature is `async` because MathJax render is async.
 */
export async function refHoverFor(
  result: { found: true; entry: LabelRef } | { found: false },
  range?: [number, number],
  cfg?: PreviewConfig,
): Promise<RefHover | null> {
  if (!result.found) return null;
  const e = result.entry;
  const header = headerFor(e);
  const location = `${relativePath(e.file)}:${e.line + 1}`;

  let snippetBlock: string;
  if (isMathEnv(e.env) && cfg) {
    const source = wrapForRender(e.env, e.snippet);
    const r = await render({
      source,
      display: true,
      scale: cfg.scale,
      color: cfg.color,
      timeoutMs: cfg.timeoutMs,
    });
    if (r.ok) {
      snippetBlock = `![formula](${toDataUri(r.svg)})`;
    } else {
      // Render failed — fall back to the fenced block so the user at
      // least sees the source.  Same fence rule as the text path.
      const fence = fenceFor(e.snippet);
      snippetBlock = `${fence}\n${e.snippet}\n${fence}`;
    }
  } else {
    const fence = fenceFor(e.snippet);
    // The fenced block always opens/closes on its own line so the markdown
    // is portable (CommonMark requires fenced-block delimiters on their
    // own line).
    snippetBlock = `${fence}\n${e.snippet}\n${fence}`;
  }
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
