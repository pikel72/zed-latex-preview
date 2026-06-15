//! Markdown formatter for `\cite{...}` hover previews.
//!
//! Takes a `BibEntry` (from `rust_sidecar.lookup("...", "cite")`) and
//! produces the markdown block shown in the hover popup, per
//! `docs/plan-ref-cite-hover.md` Section 3.2.
//!
//! The output looks like:
//!
//! ```text
//! Lamport 1986 — *LaTeX: A Document Preparation System*
//! Authors: Leslie Lamport
//! Publisher: Addison-Wesley
//! Year: 1986
//!
//! File: refs.bib:17
//! ```

import type { BibEntry, LabelRef } from "./rpc_types.js";

// ── de-parenthesise + multi-line trim ──────────────────────────────────

/**
 * Strip one level of outer braces from a BibTeX value and collapse
 * hard-wrapped continuation lines.  The brace counter honours escapes
 * (`\\{` is literal) so we don't strip past the intended depth.
 */
export function cleanFieldValue(raw: string): string {
  let s = stripOuterBraces(raw);
  // BibTeX authors and titles are commonly broken across lines with a
  // leading-space continuation.  Collapse `\n` and `\n ` to a single
  // space; also collapse runs of whitespace inside the value.
  s = s.replace(/\s*\n\s*/g, " ").replace(/\s+/g, " ").trim();
  return s;
}

function stripOuterBraces(s: string): string {
  s = s.trim();
  if (s.length < 2 || s[0] !== "{" || s[s.length - 1] !== "}") return s;
  // Honour escapes: a `\` before `{` or `}` is a literal char.
  let depth = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === "\\") {
      i++;
      continue;
    }
    if (c === "{") depth++;
    else if (c === "}") {
      depth--;
      if (depth === 0 && i !== s.length - 1) {
        // Closing brace isn't the outer one — keep as-is.
        return s;
      }
    }
  }
  if (depth === 0) return s.slice(1, -1);
  return s;
}

// ── one-line summary ───────────────────────────────────────────────────

function authorYearLine(entry: BibEntry): string | null {
  const author = entry.fields.author && cleanFieldValue(entry.fields.author);
  const year = entry.fields.year?.trim();
  const title = entry.fields.title && cleanFieldValue(entry.fields.title);
  if (author && year && title) {
    return `${shortenAuthor(author)} ${year} — *${title}*`;
  }
  if (title) {
    if (author) return `${shortenAuthor(author)} — *${title}*`;
    if (year) return `${year} — *${title}*`;
    return `*${title}*`;
  }
  if (author) return shortenAuthor(author);
  if (year) return year;
  return null;
}

/**
 * "Leslie Lamport" rather than "Lamport, Leslie".  Used for the
 * author-year heading.  Multi-author strings are left intact
 * ("Leslie Lamport and Barbara Liskov") so the user can tell at a
 * glance whether they're looking at one or several works.
 */
function shortenAuthor(author: string): string {
  return author; // BibTeX "Last, First" is the convention; do not re-order.
}

// ── the main formatter ─────────────────────────────────────────────────

export interface CiteHover {
  contents: { kind: "markdown"; value: string };
  range?: { start: { line: number; character: number }; end: { line: number; character: number } };
}

export function citeHoverFor(
  result: { found: true; entry: BibEntry } | { found: false },
  range?: [number, number],
): CiteHover | null {
  if (!result.found) {
    return {
      contents: { kind: "markdown", value: "_(citation not found)_" },
    };
  }
  const e = result.entry;
  const lines: string[] = [];
  const heading = authorYearLine(e);
  if (heading) lines.push(heading);

  // Field lines in the order from the plan: Authors, Publisher, Year.
  // We always omit a field whose cleaned value is empty.
  const orderedFields: Array<[string, string]> = [
    ["Authors", "author"],
    ["Editor", "editor"],
    ["Publisher", "publisher"],
    ["Journal", "journal"],
    ["Booktitle", "booktitle"],
    ["Volume", "volume"],
    ["Number", "number"],
    ["Pages", "pages"],
    ["Series", "series"],
    ["Edition", "edition"],
    ["Address", "address"],
    ["DOI", "doi"],
    ["URL", "url"],
    ["Year", "year"],
  ];
  for (const [label, key] of orderedFields) {
    const raw = e.fields[key];
    if (!raw) continue;
    const cleaned = cleanFieldValue(raw);
    if (!cleaned) continue;
    // Skip "Year" if we already used it in the heading.
    if (key === "year" && heading && heading.includes(cleaned)) continue;
    // Skip "Authors" if it was used in the heading.
    if (
      key === "author" &&
      heading &&
      heading.startsWith(cleaned) &&
      cleaned === e.fields.author && cleanFieldValue(e.fields.author)
    )
      continue;
    lines.push(`${label}: ${cleaned}`);
  }

  // Abstract as a block quote if present.
  if (e.fields.abstract) {
    const abs = cleanFieldValue(e.fields.abstract);
    if (abs) {
      lines.push("");
      lines.push(`> ${abs}`);
    }
  }

  // Path header.
  lines.push("");
  lines.push(`File: ${shortPath(e.file)}:${lineForOffset(e.offset, e.file)}`);

  const out: CiteHover = {
    contents: { kind: "markdown", value: lines.join("\n") },
  };
  if (range) {
    out.range = {
      start: { line: 0, character: range[0] },
      end: { line: 0, character: range[1] },
    };
  }
  return out;
}

// Re-export so the dispatch in hover.ts can also use this formatter for
// a synthetic "not found" message.
export function notFoundCiteHover(): CiteHover {
  return {
    contents: { kind: "markdown", value: "_(citation not found)_" },
  };
}

// ── small helpers ──────────────────────────────────────────────────────

function shortPath(p: string): string {
  // Trim long absolute paths down to last 2 segments so the markdown
  // doesn't push the hover popup wide.
  const norm = p.replace(/\\/g, "/");
  const parts = norm.split("/");
  if (parts.length <= 3) return norm;
  return ".../" + parts.slice(-2).join("/");
}

/**
 * Best-effort line lookup.  The Rust sidecar stores the byte offset of the
 * `@article{` opener.  We don't have the file text on the Node side, so
 * we default to line 1.  Phase-2 can plumb the source through if the
 * user complains.
 */
function lineForOffset(_offset: number, _file: string): number {
  return 1;
}

// Re-export for `ref_hover.ts` callers.
export type { LabelRef };
