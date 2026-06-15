//! User‑macro extraction and expansion.
//!
//! LaTeX documents can define custom commands via several mechanisms:
//!
//! ```tex
//! \newcommand{\R}{\mathbb{R}}           % zero‑argument
//! \newcommand*{\R}{\mathbb{R}}          % starred (no paragraph breaks in body)
//! \renewcommand{\R}{\mathbb{R}}         % redefine existing
//! \providecommand{\R}{\mathbb{R}}       % define only if undefined
//! \def\R{\mathbb{R}}                    % plain‑TeX definition
//! \DeclareMathOperator{\div}{div}       % math‑operator wrapper
//! ```
//!
//! ## Usage
//!
//! ```ts
//! const macros = extractMacros(documentText);
//! const expanded = expand("$\\R^2$", macros);  // → $\\mathbb{R}^2$
//! ```
//!
//! ## Whitelist
//!
//! Only the six defining commands above are recognised.  Arbitrary LaTeX
//! patterns that *look* like a definition (e.g. `\end{equation}\eps{3}`)
//! are NOT captured — the scanner must match one of the defining‑command
//! names or the match is discarded.
//!
//! ## Brace handling
//!
//! Macro bodies and arguments may contain nested braces (e.g.
//! `\newcommand{\x}{\sqrt{\frac{a}{b}}}`).  Regex‑based parsing cannot
//! handle arbitrary nesting, so this module walks the source with an
//! explicit brace counter for bodies and arguments.

export type MacroMap = Record<string, string>;

// ── defining‑command whitelist ─────────────────────────────────────────
// Order matters: the longer / starred forms must be tried before their
// bare prefix (`newcommand*` before `newcommand`).

const DEFINING_CMDS = [
  "newcommand*", "newcommand",
  "renewcommand*", "renewcommand",
  "providecommand*", "providecommand",
  "def",
  "DeclareMathOperator",
];

// ── brace-balanced helpers ────────────────────────────────────────────

/** Read a balanced `{…}` starting at index `at` (which must point at `{`).
 *  Returns the inner content (without the outer braces) and the offset
 *  just past the closing `}`.  Returns `null` if the braces never balance. */
function readBalancedBraces(src: string, at: number): { body: string; end: number } | null {
  if (src[at] !== "{") return null;
  let depth = 1;
  for (let i = at + 1; i < src.length; i++) {
    const ch = src[i];
    if (ch === "\\") { i++; continue; }            // skip escaped char
    if (ch === "{") depth++;
    else if (ch === "}") { depth--; if (depth === 0) return { body: src.slice(at + 1, i), end: i + 1 }; }
  }
  return null;
}

/** Skip spaces and tabs starting at `at`.  Returns the next non‑space
 *  offset. */
function skipWs(src: string, at: number): number {
  let i = at;
  while (i < src.length && (src[i] === " " || src[i] === "\t")) i++;
  return i;
}

// ── extraction ────────────────────────────────────────────────────────

/** Try to parse a macro definition starting at `at` (which must point at
 *  the leading `\`).  Returns `{ cmd, name, arity, bodyOpen }` on success
 *  where `bodyOpen` is the offset of the body's opening `{`, or `null` if
 *  the text at `at` is not one of the defining commands. */
function readMacroDef(src: string, at: number):
  { cmd: string; name: string; arity: number; bodyOpen: number } | null {
  // The character at `at` must be `\`.
  if (src[at] !== "\\") return null;

  // Try each defining command in turn.  The longest match wins so starred
  // forms (`\newcommand*`) are not misread as their bare prefix.
  let cmd: string | null = null;
  let after: number = at + 1;
  for (const c of DEFINING_CMDS) {
    if (src.startsWith(c, after) && !/[A-Za-z]/.test(src[after + c.length] ?? "")) {
      cmd = c;
      after += c.length;
      break;
    }
  }
  if (cmd === null) return null;

  after = skipWs(src, after);

  // Read the macro name: either `{\name}` (newcommand family) or `\name`
  // (plain \def).  Both forms are recognised for every command.
  let name: string;
  if (src[after] === "{") {
    const close = src.indexOf("}", after + 1);
    if (close < 0 || src[after + 1] !== "\\") return null;
    name = src.slice(after + 2, close);
    after = close + 1;
  } else if (src[after] === "\\") {
    let end = after + 1;
    while (end < src.length && /[A-Za-z@]/.test(src[end])) end++;
    name = src.slice(after + 1, end);
    after = end;
  } else {
    return null;
  }

  // Optional `[N]` arity.
  after = skipWs(src, after);
  let arity = 0;
  if (src[after] === "[") {
    const close = src.indexOf("]", after + 1);
    if (close < 0) return null;
    arity = Number(src.slice(after + 1, close));
    after = close + 1;
  }

  // Body must follow as `{…}`.
  after = skipWs(src, after);
  if (src[after] !== "{") return null;
  return { cmd, name, arity, bodyOpen: after };
}

/** Extract all user‑defined macros from a LaTeX document text. */
export function extractMacros(text: string): MacroMap {
  const out: MacroMap = {};
  let i = 0;
  while (i < text.length) {
    if (text[i] === "\\") {
      const def = readMacroDef(text, i);
      if (def) {
        const body = readBalancedBraces(text, def.bodyOpen);
        if (body) {
          let bodyStr = body.body;
          if (def.cmd === "DeclareMathOperator") {
            bodyStr = `\\operatorname{${bodyStr}}`;
          }
          out[def.name] = bodyStr;
          out[`__arity__${def.name}`] = String(def.arity);
          i = body.end;
          continue;
        }
      }
    }
    i++;
  }
  return out;
}

/**
 * Merge two macro maps, returning a new map.
 * `overrides` take precedence — macros defined there overwrite same‑named
 * macros in `base`.
 */
export function mergeMacros(base: MacroMap, overrides: MacroMap): MacroMap {
  return { ...base, ...overrides };
}

// ── expansion ─────────────────────────────────────────────────────────

/** Read one macro argument starting at `at`: either a brace-delimited
 *  `{…}` (with arbitrary nesting) or a single non-whitespace token.
 *  Returns `{ arg, end }` on success. */
function readArg(src: string, at: number): { arg: string; end: number } | null {
  let i = skipWs(src, at);
  if (i >= src.length) return null;
  if (src[i] === "{") {
    const r = readBalancedBraces(src, i);
    if (!r) return null;
    return { arg: r.body, end: r.end };
  }
  // Bare token: read until whitespace, brace, or end of input.
  let end = i;
  while (end < src.length && src[end] !== " " && src[end] !== "\t" && src[end] !== "{") end++;
  return { arg: src.slice(i, end), end };
}

/**
 * Expand every known macro in `source`, returning the expanded LaTeX string.
 *
 * Macros without arguments are substituted via `\name\b` boundary‑match.
 * Macros with arguments honour `#1`, `#2`, … placeholders; each argument may
 * be brace‑delimited (`{arg}`, with arbitrary nesting) or a single
 * non‑whitespace token (`\S`).
 */
export function expand(source: string, macros: MacroMap): string {
  let out = source;
  for (const [name, raw] of Object.entries(macros)) {
    if (name.startsWith("__arity__")) continue;
    const arity = Number(macros[`__arity__${name}`] ?? "0");
    const head = `\\${name}`;

    // Walk the source left-to-right, replacing each occurrence.  Using a
    // manual scan rather than a regex lets us call readArg (which
    // understands balanced braces) for each match.
    let result = "";
    let cursor = 0;
    while (cursor < out.length) {
      const idx = out.indexOf(head, cursor);
      if (idx < 0) {
        result += out.slice(cursor);
        break;
      }
      // Word boundary: next char must not be an identifier continuation.
      const nextCh = out[idx + head.length];
      if (nextCh && /[A-Za-z@]/.test(nextCh)) {
        result += out.slice(cursor, idx + head.length);
        cursor = idx + head.length;
        continue;
      }
      result += out.slice(cursor, idx);

      if (arity === 0) {
        result += raw;
        cursor = idx + head.length;
        continue;
      }

      // N-argument macro: consume `arity` arguments after the head.
      let pos = idx + head.length;
      const args: string[] = [];
      let ok = true;
      for (let a = 0; a < arity; a++) {
        const r = readArg(out, pos);
        if (!r) { ok = false; break; }
        args.push(r.arg);
        pos = r.end;
      }
      if (!ok) {
        // Could not parse enough arguments — leave the call site untouched.
        result += out.slice(idx, pos);
        cursor = pos;
        continue;
      }
      let expansion = raw;
      for (let a = 1; a <= arity; a++) {
        expansion = expansion.replaceAll(`#${a}`, args[a - 1]);
      }
      result += expansion;
      cursor = pos;
    }
    out = result;
  }
  return out;
}