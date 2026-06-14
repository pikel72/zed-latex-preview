//! LaTeX math-region scanner.
//!
//! `findMathAt(text, offset)` is the public entry-point.  Given the full
//! buffer text and a cursor offset, it walks backward (and forward) looking
//! for one of the recognised math delimiters and returns the source slice
//! inside the delimiters along with its range and display-mode flag.
//!
//! ## Supported delimiters
//!
//! | Delimiter | Mode  | Notes |
//! |-----------|-------|-------|
//! | `$...$`   | inline | |
//! | `\(...\)` | inline | LaTeX inline math |
//! | `$$...$$` | display | |
//! | `\[...\]` | display | LaTeX display math |
//! | `\begin{equation}...\end{equation}` | display | also `align`, etc. |
//!
//! ## Edge cases handled
//!
//! - **Escapes**: `\$` / `\\$` are not delimiters.
//! - **Parity**: `$` toggles math mode — a closing `$` is never mistaken
//!   for an opener (and vice versa for `$$`).
//! - **Comments**: `$` inside `%`-line-comments or `\begin{verbatim}` blocks
//!   is ignored and does not affect parity counting.
//! - **Cross-capture**: adjacent `$a$ text $b$` regions are kept separate;
//!   the scanner never pairs an opener with a closer that belongs to a
//!   different pair.
//! - **Unclosed `$`**: if no closing `$` is found the rest of the text is
//!   treated as math so the hover can show a TeX fallback.

export interface Position { line: number; character: number }
export interface Range { start: Position; end: Position }
export interface MathRegion { source: string; range: Range; display: boolean }

interface ScanOptions { maxFormulaLength?: number }

// ── known math environments ────────────────────────────────────────────

const ENV_NAMES = new Set([
  "equation", "equation*",
  "align", "align*",
  "gather", "gather*",
  "multline", "multline*",
]);

// ── inline verbatim / skip environments ────────────────────────────────

/** Blocks whose *entire interior* is literal text — no math, no macros. */
const SKIP_ENVS = ["verbatim", "lstlisting", "minted", "tikzpicture", "tabular"];

/** Subset of `SKIP_ENVS` where `%` is also literal (not a comment marker). */
const COMMENT_SKIPPING_ENVS = ["verbatim", "lstlisting", "minted"];

// ── low‑level char helpers ─────────────────────────────────────────────

function isAsciiWs(ch: string): boolean {
  return ch === " " || ch === "\t" || ch === "\n" || ch === "\r";
}

/** True when `src[i]` is a `$` preceded by an odd number of backslashes. */
function isEscapedDollar(src: string, i: number): boolean {
  let bs = 0;
  for (let k = i - 1; k >= 0 && src[k] === "\\"; k--) bs++;
  return bs % 2 === 1;
}

function findLineStart(src: string, offset: number): number {
  let k = offset;
  while (k > 0 && src[k - 1] !== "\n") k--;
  return k;
}

// ── comment detection ──────────────────────────────────────────────────

/**
 * True when `offset` sits on a line that starts with an un‑escaped `%`.
 * Only scans characters *before* `offset` — a `%` at `offset` itself is
 * NOT treated as a comment marker (preserves hover behaviour for the
 * cursor‑on‑`%` case).
 */
function linePercentComment(src: string, offset: number): boolean {
  const lineStart = findLineStart(src, offset);
  for (let k = lineStart; k < offset; k++) {
    if (src[k] === "%" && (k === lineStart || (!isAsciiWs(src[k - 1]) && src[k - 1] !== "\\"))) {
      return true;
    }
    // `%` inside a COMMENT_SKIPPING_ENVS block is literal, so the area
    // after `\begin{verbatim}` is NOT a comment.
    if (src[k] === "\\") {
      for (const env of COMMENT_SKIPPING_ENVS) {
        const tag = `\\begin{${env}}`;
        if (src.startsWith(tag, k)) return true;
      }
    }
  }
  return false;
}

/** True when `offset` is inside a `\begin{...}...\end{...}` skip block. */
function findEnclosingSkipEnv(text: string, offset: number): boolean {
  for (const env of SKIP_ENVS) {
    const open = `\\begin{${env}}`;
    const close = `\\end{${env}}`;
    let from = 0;
    while (true) {
      const start = text.indexOf(open, from);
      if (start < 0) break;
      const end = text.indexOf(close, start + open.length);
      if (end < 0) break;
      const endClose = end + close.length;
      if (start <= offset && offset < endClose) return true;
      from = endClose;
    }
  }
  return false;
}

// ── range construction ─────────────────────────────────────────────────

function makeRange(text: string, startOff: number, endOff: number): Range {
  return {
    start: offsetToPosition(text, startOff),
    end: offsetToPosition(text, endOff),
  };
}

// ── bracket / paren finders ────────────────────────────────────────────

/** Find `\[` at or before `offset`. Returns position of `\`, or null. */
function findOpenBracket(text: string, offset: number): number | null {
  for (let i = offset + 1; i >= 0; i--) {
    if (i < text.length && text[i] === "[" && i > 0 && text[i - 1] === "\\" && !isEscapedDollar(text, i - 1)) {
      if (linePercentComment(text, i - 1)) continue;
      return i - 1; // position of the backslash
    }
  }
  return null;
}

/** Find `\(` at or before `offset`. Returns position of `\`, or null. */
function findOpenParen(text: string, offset: number): number | null {
  for (let i = offset + 1; i >= 0; i--) {
    if (i < text.length && text[i] === "(" && i > 0 && text[i - 1] === "\\" && !isEscapedDollar(text, i - 1)) {
      if (linePercentComment(text, i - 1)) continue;
      return i - 1;
    }
  }
  return null;
}

// ── comment‑span helpers ───────────────────────────────────────────────
// A Span marks a range of text [start, end) that is "inert" — the
// characters inside should not participate in math detection.  We
// pre‑compute all such spans once per document to avoid repeated linear
// scans of the comment‑detection logic.

interface Span { start: number; end: number }

let spanCacheText: string | undefined;
let spanCache: Span[] | undefined;

function buildCommentSpans(text: string): Span[] {
  const spans: Span[] = [];

  // 1. \\begin{env}…\\end{env} blocks
  for (const env of SKIP_ENVS) {
    const openTag = `\\begin{${env}}`;
    const closeTag = `\\end{${env}}`;
    let from = 0;
    while (true) {
      const start = text.indexOf(openTag, from);
      if (start < 0) break;
      const end = text.indexOf(closeTag, start + openTag.length);
      if (end < 0) break;
      spans.push({ start, end: end + closeTag.length });
      from = end + closeTag.length;
    }
  }

  // 2. % line comments (only outside COMMENT_SKIPPING_ENVS)
  const commentSkip: Span[] = [];
  for (const env of COMMENT_SKIPPING_ENVS) {
    const openTag = `\\begin{${env}}`;
    const closeTag = `\\end{${env}}`;
    let from = 0;
    while (true) {
      const start = text.indexOf(openTag, from);
      if (start < 0) break;
      const end = text.indexOf(closeTag, start + openTag.length);
      if (end < 0) break;
      commentSkip.push({ start, end: end + closeTag.length });
      from = end + closeTag.length;
    }
  }
  const inCommentSkip = (pos: number): boolean =>
    commentSkip.some(s => s.start <= pos && pos < s.end);

  let lineStart = 0;
  for (let i = 0; i <= text.length; i++) {
    if (i === text.length || text[i] === "\n") {
      for (let k = lineStart; k < i; k++) {
        if (text[k] !== "%") continue;
        if (inCommentSkip(k)) continue;
        if (k === lineStart || (!isAsciiWs(text[k - 1]) && text[k - 1] !== "\\")) {
          spans.push({ start: k, end: i });
          break; // first unescaped % controls the rest of the line
        }
      }
      lineStart = i + 1;
    }
  }

  spans.sort((a, b) => a.start - b.start);
  return spans;
}

function getCommentSpans(text: string): Span[] {
  if (spanCacheText === text && spanCache !== undefined) return spanCache;
  spanCacheText = text;
  spanCache = buildCommentSpans(text);
  return spanCache;
}

function inComment(spans: Span[], pos: number): boolean {
  return spans.some(s => s.start <= pos && pos < s.end);
}

// ── $ helpers ──────────────────────────────────────────────────────────

/** Count non‑escaped single `$` from 0 to `end` (exclusive).  `$$` pairs
 *  are skipped.  `$` inside a comment or verbatim block is ignored so it
 *  does not perturb the math‑mode toggle parity. */
function countDollarsBefore(text: string, end: number, spans: Span[]): number {
  let count = 0;
  for (let i = 0; i < end; i++) {
    if (text[i] !== "$") continue;
    if (i + 1 < end && text[i + 1] === "$") { i++; continue; }
    if (isEscapedDollar(text, i)) continue;
    if (inComment(spans, i)) continue;
    count++;
  }
  return count;
}

/** Find an OPENING `$` at or before `offset`.  Uses `$` parity and comment
 *  awareness so a closing `$` or a `$` inside a comment is never returned. */
function findOpenDollar(text: string, offset: number, spans: Span[]): number | null {
  for (let i = offset; i >= 0; i--) {
    if (text[i] !== "$") continue;
    if (i + 1 < text.length && text[i + 1] === "$") { i--; continue; }
    if (isEscapedDollar(text, i)) continue;
    if (inComment(spans, i)) continue;
    // $ toggles math mode: even count of preceding $ → this is an opening $.
    if (countDollarsBefore(text, i, spans) % 2 === 1) continue;
    return i;
  }
  return null;
}

/** Find the matching CLOSING `$` for an inline‑math region opened at
 *  `openPos`.  Skips `$$`, escaped `\$`, and comment spans. */
function findClosingDollar(text: string, openPos: number, spans: Span[]): number | null {
  for (let i = openPos + 1; i < text.length; i++) {
    if (text[i] !== "$") continue;
    if (i + 1 < text.length && text[i + 1] === "$") { i++; continue; }
    if (isEscapedDollar(text, i)) continue;
    if (inComment(spans, i)) continue;
    return i;
  }
  return null;
}

/** Find an opening `$` at or after `offset`.  Returns `{open, close}` or
 *  null.  Used as a fallback when the backward search finds nothing and the
 *  cursor might sit *before* the opening `$`. */
function findForwardDollar(text: string, offset: number, spans: Span[]): { open: number; close: number } | null {
  for (let i = offset; i < text.length; i++) {
    if (text[i] !== "$") continue;
    if (i + 1 < text.length && text[i + 1] === "$") { i++; continue; }
    if (isEscapedDollar(text, i)) continue;
    if (inComment(spans, i)) continue;
    // Only match opening $ (even count of preceding $).
    if (countDollarsBefore(text, i, spans) % 2 === 1) continue;
    const close = findClosingDollar(text, i, spans);
    if (close !== null && close > i + 1) {
      return { open: i, close };
    }
    return null;
  }
  return null;
}

// ── $$ helpers ─────────────────────────────────────────────────────────

/** Count `$$` pairs from 0 to `end`.  `$$` toggles display‑math mode just
 *  like `$` toggles inline‑math mode — even count before a `$$` means it
 *  is an opener. */
function countDollarDollarPairsBefore(text: string, end: number, spans: Span[]): number {
  let count = 0;
  for (let i = 0; i < end; i++) {
    if (text[i] !== "$" || i + 1 >= end || text[i + 1] !== "$") continue;
    if (isEscapedDollar(text, i)) { i++; continue; }
    if (inComment(spans, i)) { i++; continue; }
    count++;
    i++; // skip the second `$`
  }
  return count;
}

/** Find an OPENING `$$` at or before `offset`.  Uses `$$` parity so a
 *  closing `$$` (odd pair count) is never returned. */
function findOpenDollarDollar(text: string, offset: number, spans: Span[]): number | null {
  for (let i = offset; i >= 0; i--) {
    if (text[i] !== "$" || i + 1 >= text.length || text[i + 1] !== "$") continue;
    if (isEscapedDollar(text, i)) continue;
    if (inComment(spans, i)) continue;
    if (countDollarDollarPairsBefore(text, i, spans) % 2 === 1) continue;
    return i;
  }
  return null;
}

/** Find the closing `$$` for a display‑math region opened at `openPos`. */
function findClosingDollarDollar(text: string, openPos: number, spans: Span[]): number | null {
  for (let i = openPos + 2; i < text.length; i++) {
    if (text[i] !== "$" || i + 1 >= text.length || text[i + 1] !== "$") continue;
    if (isEscapedDollar(text, i)) continue;
    if (inComment(spans, i)) continue;
    return i;
  }
  return null;
}

// ── environment finder ─────────────────────────────────────────────────

/** Find the nearest `\begin{env}...\end{env}` enclosing `offset`. */
function findOpenEnv(
  text: string,
  offset: number,
  env: string,
): { open: number; sourceStart: number; close: number; sourceEnd: number; closeEnd: number } | null {
  const tag = `\\begin{${env}}`;
  const closeTag = `\\end{${env}}`;
  let best: ReturnType<typeof findOpenEnv> = null;
  let from = 0;
  while (true) {
    const open = text.indexOf(tag, from);
    if (open < 0) break;
    if (open > offset) break;
    if (linePercentComment(text, open)) { from = open + tag.length; continue; }
    const sourceStart = open + tag.length;
    const close = text.indexOf(closeTag, sourceStart);
    if (close < 0) break;
    const sourceEnd = close;
    const closeEnd = close + closeTag.length;
    if (sourceStart <= offset && offset <= sourceEnd) {
      if (!best || open > best.open) best = { open, sourceStart, close, sourceEnd, closeEnd };
    }
    from = closeEnd;
  }
  return best;
}

// ── public API ─────────────────────────────────────────────────────────

/**
 * Given the full buffer `text` and a cursor offset, locate the math region
 * (if any) that contains the cursor and return its source, range, and
 * display‑mode flag.
 *
 * The delimiters are checked in priority order: `\[…\]`, `\(…\)`, `$$…$$`,
 * `$…$` (backward, then forward), and finally `\begin{env}…\end{env}`.
 * Only the first match that fully encloses the cursor is returned.
 */
export function findMathAt(text: string, offset: number, opts: ScanOptions = {}): MathRegion | null {
  const max = opts.maxFormulaLength ?? 2000;

  // 0. Bail out early if the cursor is inside a verbatim/tabular/etc. block.
  if (findEnclosingSkipEnv(text, offset)) return null;

  const spans = getCommentSpans(text);

  // 1. \[ ... \]
  {
    const open = findOpenBracket(text, offset);
    if (open !== null) {
      const close = text.indexOf("\\]", open + 2);
      if (close > offset) {
        const src = text.slice(open + 2, close);
        if (src.length <= max) {
          return { source: src, range: makeRange(text, offset, close + 2), display: true };
        }
      }
    }
  }

  // 2. \( ... \)
  {
    const open = findOpenParen(text, offset);
    if (open !== null) {
      const close = text.indexOf("\\)", open + 2);
      if (close > offset) {
        const src = text.slice(open + 2, close);
        if (src.length <= max) {
          const endOff = (offset === open) ? close : close + 2;
          return { source: src, range: makeRange(text, offset, endOff), display: false };
        }
      }
    }
  }

  // 3. $$ ... $$
  {
    const open = findOpenDollarDollar(text, offset, spans);
    if (open !== null) {
      const close = findClosingDollarDollar(text, open, spans);
      if (close !== null && close > offset) {
        const src = text.slice(open + 2, close);
        if (src.length <= max) {
          return { source: src, range: makeRange(text, offset, close + 2), display: true };
        }
      }
    }
  }

  // 4. $ ... $ (backward search)
  {
    const open = findOpenDollar(text, offset, spans);
    if (open !== null) {
      const close = findClosingDollar(text, open, spans);
      // An unclosed `$` (close === null) is malformed LaTeX — treat the
      // rest of the text as math so the hover can show a TeX fallback.
      const effClose = close ?? text.length;
      if ((close !== null && close > offset && close > open + 1) || (close === null && open < offset)) {
        const src = text.slice(open + 1, effClose);
        if (src.length <= max) {
          const endOff = (offset === open) ? effClose : (close !== null ? close + 1 : effClose);
          return { source: src, range: makeRange(text, offset, endOff), display: false };
        }
      }
    }
  }

  // 5. $ ... $ (forward search — cursor before the opening `$`)
  {
    const fwd = findForwardDollar(text, offset, spans);
    if (fwd !== null) {
      const src = text.slice(fwd.open + 1, fwd.close);
      if (src.length <= max) {
        return { source: src, range: makeRange(text, offset, fwd.open), display: false };
      }
    }
  }

  // 6. \begin{env}...\end{env}
  for (const env of ENV_NAMES) {
    const found = findOpenEnv(text, offset, env);
    if (found !== null) {
      const src = text.slice(found.sourceStart, found.sourceEnd);
      if (src.length <= max) {
        const endOff = (offset === found.sourceStart) ? found.close + 1 : found.sourceEnd + src.length;
        return { source: src, range: makeRange(text, offset, endOff), display: true };
      }
    }
  }

  return null;
}

/** Convert a byte offset to a 0‑based line+character position. */
export function offsetToPosition(text: string, offset: number): Position {
  let line = 0, ch = 0;
  for (let i = 0; i < offset && i < text.length; i++) {
    if (text[i] === "\n") { line++; ch = 0; } else ch++;
  }
  return { line, character: ch };
}
