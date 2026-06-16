//! User‑facing configuration (from Zed's `lsp.latex-preview.settings`).
//!
//! Settings are forwarded to the LSP as `initializationOptions` by the Rust
//! side of the extension.  This module defines the TypeScript shape and a
//! `configFromInit` factory that fills defaults for any missing keys.

export type ColorMode = "auto" | "black" | "white";
export type Renderer = "mathjax";

export interface PreviewConfig {
  enabled: boolean;
  maxFormulaLength: number;
  timeoutMs: number;
  scale: number;
  color: ColorMode;
  renderer: Renderer;
  // ── new for the sidecar split (Phase 1) ──────────────────────────
  /** When true, the Node LSP spawns the Rust `latex-index` sidecar
   *  on `onInitialize`.  Falls back to the in-process extractor on
   *  spawn failure.  Default `true`. */
  enabledSidecar: boolean;
  /** When true, hover previews are shown for `\cite{...}`.  Default `true`. */
  enabledCitePreview: boolean;
  /** When true, hover previews are shown for `\ref{...}`.  Default `true`. */
  enabledRefPreview: boolean;
  /** When true, hover previews are shown for `\usepackage{<pkg>}` and
   *  bare command names (e.g. `\textbf`) that have an entry in the
   *  bundled dictionary.  Default `true`.  Independent of the cite / ref
   *  flags so users can opt out without losing ref preview. */
  enabledDocPreview: boolean;
  /** Optional explicit path to the `latex-index` binary.  Default `null`
   *  = auto-resolve from `LATEX_INDEX_PATH`, the cargo `target/` dir, or PATH. */
  sidecarPath: string | null;
  /** Maximum size of a `.bib` file to parse, in MiB.  Default `5`. */
  bibMaxFileSizeMB: number;
}

export function defaultConfig(): PreviewConfig {
  return {
    enabled: true,
    maxFormulaLength: 2000,
    timeoutMs: 1500,
    scale: 1.4,
    color: "auto",
    renderer: "mathjax",
    enabledSidecar: true,
    enabledCitePreview: true,
    enabledRefPreview: true,
    enabledDocPreview: true,
    sidecarPath: null,
    bibMaxFileSizeMB: 5,
  };
}

/** Build a `PreviewConfig` from the opaque `initializationOptions` blob
 *  received from the client.  Unknown keys are silently ignored; missing
 *  keys fall back to `defaultConfig()`. */
export function configFromInit(initializationOptions: unknown): PreviewConfig {
  const cfg = defaultConfig();
  const o = initObject(initializationOptions);
  if (!o) return cfg;
  cfg.enabled    = pick(o, "enabled",    isBool,   cfg.enabled);
  cfg.scale      = pick(o, "scale",      isNumber, cfg.scale);
  cfg.color      = pick(o, "color",      isColor,  cfg.color);
  cfg.timeoutMs  = pick(o, "timeoutMs",  isNumber, cfg.timeoutMs);
  cfg.maxFormulaLength = pick(o, "maxFormulaLength", isNumber, cfg.maxFormulaLength);
  cfg.renderer   = pick(o, "renderer",   isRenderer, cfg.renderer);
  // Phase-1 keys (all optional, with safe defaults).
  cfg.enabledSidecar    = pick(o, "enabledSidecar",    isBool,  cfg.enabledSidecar);
  cfg.enabledCitePreview = pick(o, "enabledCitePreview", isBool, cfg.enabledCitePreview);
  cfg.enabledRefPreview  = pick(o, "enabledRefPreview",  isBool, cfg.enabledRefPreview);
  cfg.enabledDocPreview  = pick(o, "enabledDocPreview",  isBool,  cfg.enabledDocPreview);
  cfg.sidecarPath       = pick(o, "sidecarPath",       isString, cfg.sidecarPath);
  cfg.bibMaxFileSizeMB  = pick(o, "bibMaxFileSizeMB",  isNumber, cfg.bibMaxFileSizeMB);
  return cfg;
}

// ── guards + helpers ──────────────────────────────────────────────────

function isBool(v: unknown): v is boolean { return typeof v === "boolean"; }
function isNumber(v: unknown): v is number { return typeof v === "number"; }
function isString(v: unknown): v is string { return typeof v === "string"; }
function isColor(v: unknown): v is ColorMode {
  return v === "auto" || v === "black" || v === "white";
}
function isRenderer(v: unknown): v is Renderer { return v === "mathjax"; }

function initObject(v: unknown): Record<string, unknown> | null {
  if (!v || typeof v !== "object") return null;
  return v as Record<string, unknown>;
}

function pick<T>(o: Record<string, unknown>, k: string, guard: (v: unknown) => v is T, fallback: T): T {
  return guard(o[k]) ? o[k] : fallback;
}