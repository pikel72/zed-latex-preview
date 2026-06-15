//! Markdown formatter for the package/command dictionary hover preview.
//!
//! When the cursor is on `\usepackage{<pkg>}`, `\RequirePackage{<pkg>}`,
//! a bare command like `\textbf`, or an env name inside `\begin{<env>}` /
//! `\end{<env>}`, `cursor.rs` returns `kind: "doc"` and the dispatcher in
//! `hover.ts` calls into this module.  We ask the Rust sidecar for the
//! bundled dictionary entry and render it as markdown.
//!
//! Per spec §4.9, the rendered shape is:
//!
//! ```text
//! **amsmath** (package)
//!
//! <short>     // or <docs> when present
//! ```
//!
//! Returns `null` when `found: false` so the dispatcher can fall through
//! to the math path.

import type { SidecarHandle } from "./rust_sidecar.js";

// ── types ──────────────────────────────────────────────────────────────

export interface DocHover {
  contents: { kind: "markdown"; value: string };
}

// ── the main formatter ─────────────────────────────────────────────────

export async function docHoverFor(
  name: string,
  sidecar: SidecarHandle,
): Promise<DocHover | null> {
  let r;
  try {
    r = await sidecar.doc_lookup(name);
  } catch {
    return null;
  }
  if (!r.found) return null;
  const e = r.entry;
  // Spec §4.9: kind tag is rendered as "package" or "command".
  const kindLabel = e.kind === "package" ? "package" : "command";
  const header = `**${e.title}** (${kindLabel})`;
  const body = e.docs && e.docs.length > 0 ? e.docs : e.short;
  const value = `${header}\n\n${body}`;
  return {
    contents: { kind: "markdown", value },
  };
}
