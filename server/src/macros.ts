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
//! are NOT captured — the regex must match one of the defining‑command
//! names or the match is discarded.

export type MacroMap = Record<string, string>;

// ── defining‑command whitelist ─────────────────────────────────────────
// Everything else that happens to match the regex below is a false positive
// and must be discarded.

const DEFINING_CMDS = new Set([
  "newcommand", "newcommand*",
  "renewcommand", "renewcommand*",
  "providecommand", "providecommand*",
  "def",
  "DeclareMathOperator",
]);

// ── extraction regex ───────────────────────────────────────────────────
//
// Captures groups:
//   1. command   – the defining command (e.g. "newcommand", "def")
//   2. name      – the macro name *without* leading backslash ("R")
//   3. arity?    – optional argument count in square brackets
//   4. body      – the replacement text in curly braces

const RE_NEWCOMMAND = /\\([A-Za-z]+\*?)\s*\{?\\([A-Za-z@]+)\}?\s*(?:\[(\d+)\])?\s*\{((?:[^{}]|\{[^{}]*\})*)\}/g;

/** Extract all user‑defined macros from a LaTeX document text. */
export function extractMacros(text: string): MacroMap {
  const out: MacroMap = {};
  RE_NEWCOMMAND.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = RE_NEWCOMMAND.exec(text)) !== null) {
    const cmd = m[1];
    if (!DEFINING_CMDS.has(cmd)) continue;   // false positive — discard
    const name = m[2];
    const arity = m[3] ? Number(m[3]) : 0;
    let body = m[4];
    if (cmd === "DeclareMathOperator") {
      body = `\\operatorname{${body}}`;
    }
    out[name] = body;
    out[`__arity__${name}`] = String(arity);
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

/**
 * Expand every known macro in `source`, returning the expanded LaTeX string.
 *
 * Macros without arguments are substituted via `\name\b` boundary‑match.
 * Macros with arguments honour `#1`, `#2`, … placeholders; each argument may
 * be brace‑delimited (`{arg}`) or a single non‑whitespace token (`\S`).
 */
export function expand(source: string, macros: MacroMap): string {
  let out = source;
  for (const [name, raw] of Object.entries(macros)) {
    if (name.startsWith("__arity__")) continue;
    const arity = Number(macros[`__arity__${name}`] ?? "0");
    if (arity === 0) {
      // Zero‑argument macro: replace \name (word‑boundary to avoid
      // partial matches, e.g. \Re should not match \R).
      const re = new RegExp(`\\\\${name}\\b`, "g");
      out = out.replace(re, raw);
    } else {
      // N‑argument macro: match \name followed by N argument groups.
      // Each group is either {arg} or a single non‑whitespace token.
      const argPat = `(?:\\{\\s*([^{}]*)\\s*\\}|(\\S))`;
      const re = new RegExp(`\\\\${name}${argPat.repeat(arity)}`, "g");
      out = out.replace(re, (...args) => {
        // Capture groups come as args[1..2*arity], paired (brace, bare).
        const groups = args.slice(1, 1 + 2 * arity);
        const captured: string[] = [];
        for (let i = 0; i < arity; i++) {
          const brace = groups[2 * i];     // brace-delimited capture
          const bare = groups[2 * i + 1];  // single-token capture
          captured.push(brace ?? bare ?? "");
        }
        let body = raw;
        for (let i = 1; i <= arity; i++) {
          body = body.replaceAll(`#${i}`, captured[i - 1]);
        }
        return body;
      });
    }
  }
  return out;
}
