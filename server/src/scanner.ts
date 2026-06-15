//! LaTeX math-region scanner.
//!
//! `findMathAt(text, offset)` is the public entry point.  Given the full
//! buffer text and a cursor offset, it returns the math region (if any) that
//! contains the cursor: its source (inside the delimiters), its full range
//! (spanning both delimiters), and a display-mode flag.
//!
//! ## Implementation
//!
//! A single left-to-right pass builds a list of every math region in the
//! document, skipping `%` comments and verbatim-class environments.  The pass
//! is O(n) in the document length, and the result is cached per text so
//! repeated hovers on the same buffer are O(log) lookups.
//!
//! ## Supported delimiters
//!
//! | Delimiter | Mode |
//! |-----------|------|
//! | `$…$` | inline |
//! | `\(…\)` | inline |
//! | `$$…$$` | display |
//! | `\[…\]` | display |
//! | `\begin{equation}…\end{equation}` (and `align`, `gather`, `multline`, + starred) | display |
//!
//! Escapes (`\$`), comments (`%`), and verbatim environments are recognised
//! so delimiters inside them do not toggle math mode.

export interface Position { line: number; character: number }
export interface Range { start: Position; end: Position }
export interface MathRegion { source: string; range: Range; display: boolean }

interface ScanOptions { maxFormulaLength?: number }

/** A single math region with its full byte offsets (delimiters included). */
interface MathSpan {
  openDelim: number;   // offset of the opening delimiter
  endDelim: number;    // offset just past the closing delimiter
  source: string;      // text inside the delimiters
  display: boolean;
}

// ── environment tables ─────────────────────────────────────────────────

/** Math environments whose body is a display-math region. */
const MATH_ENVS = new Set([
  "equation", "equation*",
  "align", "align*",
  "gather", "gather*",
  "multline", "multline*",
]);

/** Environments whose entire interior is literal text — no math, no macros. */
const VERBATIM_ENVS = new Set(["verbatim", "lstlisting", "minted"]);

// ── span cache (per text) ──────────────────────────────────────────────

let cacheText: string | undefined;
let cacheSpans: MathSpan[] | undefined;

function getSpans(text: string): MathSpan[] {
  if (cacheText === text && cacheSpans !== undefined) return cacheSpans;
  cacheText = text;
  cacheSpans = tokenize(text);
  return cacheSpans;
}

/** Drop the cached spans (e.g. when a document is closed). */
export function invalidateScannerCache(): void {
  cacheText = undefined;
  cacheSpans = undefined;
}

// ── low-level helpers ──────────────────────────────────────────────────

/** True when `src[i]` is preceded by an odd number of backslashes. */
function isEscaped(src: string, i: number): boolean {
  let bs = 0;
  for (let k = i - 1; k >= 0 && src[k] === "\\"; k--) bs++;
  return bs % 2 === 1;
}

/** Read the environment name in `\begin{NAME}` / `\end{NAME}` starting at the
 *  backslash.  Returns `{ name, tagEnd }` where `tagEnd` is the offset just
 *  past the closing `}`, or `null` when the text does not match. */
function readEnvTag(src: string, at: number, kind: "begin" | "end"):
  { name: string; tagEnd: number } | null {
  const want = kind === "begin" ? "\\begin" : "\\end";
  if (!src.startsWith(want, at)) return null;
  let i = at + want.length;
  while (i < src.length && (src[i] === " " || src[i] === "\t")) i++;
  if (src[i] !== "{") return null;
  const nameEnd = src.indexOf("}", i + 1);
  if (nameEnd < 0) return null;
  return { name: src.slice(i + 1, nameEnd), tagEnd: nameEnd + 1 };
}

/**
 * Find the next dollar closer from `from` onward, skipping escapes,
 * comments, and (when `wantDouble` is false) `$$` pairs.
 * `wantDouble` true → find `$$`; false → find a single `$`.
 */
function findCloser(text: string, from: number, wantDouble: boolean): number {
  for (let i = from; i < text.length; i++) {
    const ch = text[i];
    if (ch === "\\") { i++; continue; }            // skip escaped char
    if (ch === "%") {                               // skip to EOL
      const nl = text.indexOf("\n", i);
      i = nl < 0 ? text.length : nl;
      continue;
    }
    if (ch === "$") {
      const isDouble = text[i + 1] === "$";
      if (wantDouble) { if (isDouble) return i; }
      else            { if (!isDouble) return i; i += isDouble ? 1 : 0; }
    }
  }
  return -1;
}

// ── tokenizer ──────────────────────────────────────────────────────────

/** Single left-to-right scan producing every math region in `text`. */
function tokenize(text: string): MathSpan[] {
  const spans: MathSpan[] = [];
  const n = text.length;
  let i = 0;

  /** Push a delimited region `[openDelim, endDelim)` sliced from `bodyStart..bodyEnd`. */
  const push = (openDelim: number, endDelim: number, bodyStart: number, bodyEnd: number, display: boolean) =>
    spans.push({ openDelim, endDelim, source: text.slice(bodyStart, bodyEnd), display });

  while (i < n) {
    const ch = text[i];

    // 1. Comment to end of line.
    if (ch === "%" && !isEscaped(text, i)) {
      const nl = text.indexOf("\n", i + 1);
      i = nl < 0 ? n : nl + 1;
      continue;
    }

    // 2. \begin{...} → dispatch by environment name.
    //    Verbatim envs: skip the whole body.  Math envs: emit a display region.
    if (ch === "\\" && text.startsWith("\\begin", i) && !isEscaped(text, i)) {
      const tag = readEnvTag(text, i, "begin");
      if (tag) {
        const closeTag = `\\end{${tag.name}}`;
        const close = text.indexOf(closeTag, tag.tagEnd);
        const endDelim = close < 0 ? n : close + closeTag.length;
        if (VERBATIM_ENVS.has(tag.name)) {
          i = endDelim;
          continue;
        }
        if (MATH_ENVS.has(tag.name)) {
          push(i, endDelim, tag.tagEnd, close < 0 ? n : close, true);
          i = endDelim;
          continue;
        }
      }
    }

    // 3. `$$...$$` display math (check before single `$`).
    if (ch === "$" && !isEscaped(text, i) && text[i + 1] === "$") {
      const close = findCloser(text, i + 2, true);
      push(i, close < 0 ? n : close + 2, i + 2, close < 0 ? n : close, true);
      i = close < 0 ? n : close + 2;
      continue;
    }

    // 4. `$...$` inline math.
    if (ch === "$" && !isEscaped(text, i)) {
      const close = findCloser(text, i + 1, false);
      push(i, close < 0 ? n : close + 1, i + 1, close < 0 ? n : close, false);
      i = close < 0 ? n : close + 1;
      continue;
    }

    // 5. `\( ... \)` inline and `\[ ... \]` display math.
    if (ch === "\\" && (text[i + 1] === "(" || text[i + 1] === "[") && !isEscaped(text, i)) {
      const display = text[i + 1] === "[";
      const closeDelim = display ? "\\]" : "\\)";
      const close = text.indexOf(closeDelim, i + 2);
      push(i, close < 0 ? n : close + 2, i + 2, close < 0 ? n : close, display);
      i = close < 0 ? n : close + 2;
      continue;
    }

    i++;
  }

  return spans;
}

// ── public API ─────────────────────────────────────────────────────────

/**
 * Given the full buffer `text` and a cursor offset, locate the math region
 * (if any) that contains the cursor and return its source, its full range
 * (delimiters included), and its display-mode flag.
 *
 * The returned range always spans the complete math region — `[openDelim,
 * endDelim)` — so the hover highlight does not move with the cursor.
 */
export function findMathAt(text: string, offset: number, opts: ScanOptions = {}): MathRegion | null {
  const max = opts.maxFormulaLength ?? 2000;
  for (const s of getSpans(text)) {
    if (offset >= s.openDelim && offset < s.endDelim) {
      if (s.source.length > max) return null;
      return {
        source: s.source,
        range: {
          start: offsetToPosition(text, s.openDelim),
          end: offsetToPosition(text, s.endDelim),
        },
        display: s.display,
      };
    }
  }
  return null;
}

// ── offset → position ──────────────────────────────────────────────────

/** Convert a byte offset to a 0-based line+character position.
 *  `\r` is not counted as a character (LSP positions exclude line terminators),
 *  so CRLF documents don't cause drift. */
function offsetToPosition(text: string, offset: number): Position {
  let line = 0, ch = 0;
  for (let i = 0; i < offset && i < text.length; i++) {
    if (text[i] === "\r") continue;
    if (text[i] === "\n") { line++; ch = 0; } else ch++;
  }
  return { line, character: ch };
}
