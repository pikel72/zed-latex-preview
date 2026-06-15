//! Hover handler — the core of the extension.
//!
//! On every `textDocument/hover` request the handler:
//!
//! 1. Locates the math region under the cursor with `findMathAt`.
//! 2. Merges workspace‑wide macros with the current document's own macros.
//! 3. Expands all user macros in the formula source.
//! 4. Renders the expanded source with MathJax.
//! 5. Returns either a markdown `![formula](data:…)` image or a plain‑text
//!    code block (on render failure).
//!
//! Results are cached per (expanded source, macro set, theme, scale,
//! display‑mode) key.

import { findMathAt, positionToOffset, type Position } from "./scanner.js";
import { extractMacros, expand, mergeMacros, type MacroMap } from "./macros.js";
import { render } from "./render.js";
import { LRU, memoizeByText, type CacheKey } from "./cache.js";
import type { PreviewConfig } from "./config.js";
import { getWorkspaceMacros } from "./preamble.js";
import type { SidecarHandle } from "./rust_sidecar.js";
import { citeHoverFor } from "./cite_hover.js";
import { refHoverFor } from "./ref_hover.js";

// ── types ──────────────────────────────────────────────────────────────

export interface HoverResult {
  contents: { kind: "markdown"; value: string };
  range?: { start: Position; end: Position };
}

const cache = new LRU<{ ok: true; dataUri: string } | { ok: false; error: string }>(256);

// ── per‑document macro cache ───────────────────────────────────────────
// Avoids re‑scanning the full document text for macros on every hover.

const docMacrosCache = memoizeByText(extractMacros);
const getDocMacros = docMacrosCache.get;

// ── helpers ────────────────────────────────────────────────────────────

function toDataUri(svg: string): string {
  return "data:image/svg+xml;base64," + Buffer.from(svg, "utf8").toString("base64");
}

// ── public API ─────────────────────────────────────────────────────────

/**
 * Process a hover request for a single document.
 *
 * @param text           Full buffer text of the hovered document.
 * @param position       0‑based cursor position.
 * @param cfg            Current user configuration.
 * @param uri            Document URI — required when a sidecar is present
 *                       so it can look up the cursor context in the
 *                       most-recently-seen buffer.
 * @param sidecar        The Rust sidecar handle, or `null` if it failed
 *                       to spawn (in which case we fall back to math only).
 * @param macroOverride  Testing hook — when set, workspace macros are
 *                       bypassed and this map is used instead.
 */
export async function hoverFor(
  text: string,
  position: Position,
  cfg: PreviewConfig,
  uri?: string,
  sidecar?: SidecarHandle | null,
  macroOverride?: MacroMap,
): Promise<HoverResult | null> {
  if (!cfg.enabled) return null;

  const offset = positionToOffset(text, position);

  // ── Phase 1: ask the sidecar what kind of cursor we are on. ──────
  if (
    sidecar &&
    uri &&
    (cfg.enabledCitePreview || cfg.enabledRefPreview)
  ) {
    let ctx: Awaited<ReturnType<SidecarHandle["cursor_context"]>> | null = null;
    try {
      ctx = await sidecar.cursor_context(uri, offset);
    } catch {
      // Sidecar hiccup — fall through to math.
      ctx = null;
    }
    if (ctx) {
      if (ctx.kind === "cite" && cfg.enabledCitePreview && ctx.key) {
        try {
          const r = await sidecar.lookup(ctx.key, "cite");
          return citeHoverFor(r, ctx.range);
        } catch {
          // fall through to math
        }
      } else if (ctx.kind === "ref" && cfg.enabledRefPreview && ctx.key) {
        try {
          const r = await sidecar.lookup(ctx.key, "ref");
          const out = refHoverFor(r, ctx.range);
          if (out) return out;
          // Phase-1 stub: ref-hover returns null → fall through to math.
        } catch {
          // fall through
        }
      }
      // `kind: "math"` or `kind: "none"` → fall through to the existing
      // math path so inline math still gets rendered.
    }
  }

  // ── existing math hover path ─────────────────────────────────────
  const region = findMathAt(text, offset, { maxFormulaLength: cfg.maxFormulaLength });
  if (!region) return null;

  // Workspace‑wide macros as base, then this document's own definitions
  // override them (document takes precedence over other files).
  const base = macroOverride ?? getWorkspaceMacros();
  const docMacros = getDocMacros(text);
  const macros = mergeMacros(base, docMacros);
  const expanded = expand(region.source, macros);
  const macroBlock = JSON.stringify(macros);

  const key: CacheKey = {
    source: expanded,
    macroBlock,
    theme: cfg.color,
    scale: cfg.scale,
    display: region.display,
  };

  let entry = cache.get(key);
  if (!entry) {
    try {
      const r = await render({
        source: expanded,
        display: region.display,
        scale: cfg.scale,
        color: cfg.color,
        timeoutMs: cfg.timeoutMs,
      });
      entry = r.ok
        ? { ok: true, dataUri: toDataUri(r.svg) }
        : { ok: false, error: r.error };
    } catch (e) {
      entry = { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
    cache.set(key, entry);
  }

  const md = entry.ok
    ? `![formula](${entry.dataUri})`
    : `\`\`\`latex\n${region.source}\n\`\`\``;

  return {
    contents: { kind: "markdown", value: md },
    range: region.range,
  };
}

// ── cursor‑to‑offset conversion ────────────────────────────────────────
// Imported from scanner.ts so both directions share a single CRLF-aware
// implementation.
