//! User‑facing configuration (from Zed's `lsp.latex-preview.settings`).
//!
//! Settings are serialised as JSON by the Rust side of the extension and
//! forwarded to the LSP via workspace configuration.  This module defines
//! the TypeScript shape and a `configFromInit` factory that fills defaults
//! for any missing keys.

export interface PreviewConfig {
  enabled: boolean;
  maxFormulaLength: number;
  timeoutMs: number;
  scale: number;
  color: "auto" | "black" | "white";
  renderer: "mathjax";
  /** User‑supplied preamble file paths (relative to workspace root).
   *  Kept for backward compatibility but no longer required — workspace
   *  macros are auto‑discovered. */
  preamble: string[];
}

export function defaultConfig(): PreviewConfig {
  return {
    enabled: true,
    maxFormulaLength: 2000,
    timeoutMs: 1500,
    scale: 1.4,
    color: "auto",
    renderer: "mathjax",
    preamble: [],
  };
}

function normalizePreamble(v: unknown): string[] {
  if (!v) return [];
  if (typeof v === "string") {
    return v.split(/[;,]/).map(s => s.trim()).filter(Boolean);
  }
  if (Array.isArray(v)) {
    return v.filter((x): x is string => typeof x === "string");
  }
  return [];
}

/** Build a `PreviewConfig` from the opaque `initializationOptions` blob
 *  received from the client.  Unknown keys are silently ignored; missing
 *  keys fall back to `defaultConfig()`. */
export function configFromInit(initializationOptions: unknown): PreviewConfig {
  const cfg = defaultConfig();
  if (!initializationOptions || typeof initializationOptions !== "object") return cfg;
  const o = initializationOptions as Record<string, unknown>;
  if (typeof o.enabled === "boolean") cfg.enabled = o.enabled;
  if (typeof o.maxFormulaLength === "number") cfg.maxFormulaLength = o.maxFormulaLength;
  if (typeof o.timeoutMs === "number") cfg.timeoutMs = o.timeoutMs;
  if (typeof o.scale === "number") cfg.scale = o.scale;
  if (o.color === "auto" || o.color === "black" || o.color === "white") cfg.color = o.color;
  if (o.renderer === "mathjax") cfg.renderer = o.renderer;
  if (o.preamble !== undefined) cfg.preamble = normalizePreamble(o.preamble);
  return cfg;
}
