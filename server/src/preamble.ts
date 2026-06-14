//! Workspace‑wide macro auto‑discovery.
//!
//! When the LSP initialises, `initWorkspaceMacros()` recursively walks the
//! workspace directory tree looking for every `*.tex` file (skipping known
//! noise directories like `node_modules`, `.git`, `texghost`).  Each file's
//! `\newcommand`, `\def`, … definitions are extracted and cached.
//!
//! On every `textDocument/didOpen` and `didChange` the per‑file cache is
//! updated, so macro edits are reflected immediately.
//!
//! ## Zero configuration
//!
//! The user does *not* need to list preamble files manually.  If the
//! workspace root is unknown at initialisation time (null `rootUri`), the
//! first `didOpen` triggers a lazy scan with the project root inferred
//! from the opened file's location.
//!
//! ## Merge order
//!
//! All workspace macros are merged into a single `MacroMap`.  The current
//! document's own macros (from `hover.ts`) take precedence, so in‑document
//! re‑definitions override the workspace baseline.

import * as fs from "node:fs";
import * as path from "node:path";
import { extractMacros, type MacroMap } from "./macros.js";

// ── state ──────────────────────────────────────────────────────────────

/** Per‑file cache: absolute path → { macroName → body }. */
const fileCache = new Map<string, MacroMap>();
let workspaceRoot: string | null = null;
let scanned = false;

/** Return the merged macro map from all discovered workspace files. */
export function getWorkspaceMacros(): MacroMap {
  const merged: MacroMap = {};
  for (const m of fileCache.values()) Object.assign(merged, m);
  return merged;
}

/**
 * Scan the workspace for every `*.tex` file, extract macros, and cache
 * them.  Called once on LSP initialisation.
 *
 * @param rootUri  Workspace root as a `file://` URI or plain absolute path.
 */
export function initWorkspaceMacros(rootUri: string | null): void {
  const root = normalizePath(rootUri);
  if (!root) return;
  if (root === workspaceRoot && scanned) return;  // already done
  scan(root);
}

/**
 * Update the macro cache for one specific file.  Called on every
 * `textDocument/didOpen` and `didChange`.
 *
 * If the workspace hasn't been scanned yet (rootUri was null during init),
 * the project root is inferred from `uri` and a lazy scan is triggered.
 */
export function updateFileMacros(uri: string, text: string): void {
  const filePath = normalizePath(uri);
  if (!filePath) return;

  if (!scanned) {
    // Lazy initialisation — discover the project root from this file and
    // walk upward until we find the top‑most directory containing .tex files.
    let dir = path.dirname(filePath);
    for (let i = 0; i < 3; i++) {
      const parent = path.dirname(dir);
      if (parent === dir || !hasTexFiles(parent)) break;
      dir = parent;
    }
    scan(dir);
  }

  fileCache.set(filePath, extractMacros(text));
}

// ── internal scan ──────────────────────────────────────────────────────

function scan(root: string): void {
  workspaceRoot = root;
  fileCache.clear();

  for (const abs of collectTexFiles(root)) {
    try {
      fileCache.set(abs, extractMacros(fs.readFileSync(abs, "utf-8")));
    } catch {
      // File disappeared / permission denied — skip silently.
    }
  }
  scanned = true;
}

// ── file‑system helpers ────────────────────────────────────────────────

/** Directories that are never worth scanning. */
const SKIP_DIRS = new Set([
  "node_modules", ".git", "texghost", "out", "target", "dist", "__pycache__",
]);

/** Recursively collect every `*.tex` file under `dir` (depth‑limited, skips
 *  noise dirs and dot‑dirs).  Errors reading a directory are swallowed. */
function collectTexFiles(dir: string, depth = 0, out: string[] = []): string[] {
  if (depth > 20) return out;
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out; // permission error
  }
  for (const e of entries) {
    if (e.isDirectory()) {
      if (SKIP_DIRS.has(e.name)) continue;
      if (e.name.startsWith(".") && e.name.length > 1) continue;  // .hidden
      collectTexFiles(path.join(dir, e.name), depth + 1, out);
    } else if (e.isFile() && e.name.endsWith(".tex")) {
      out.push(path.join(dir, e.name));
    }
  }
  return out;
}

function hasTexFiles(dir: string): boolean {
  try {
    return fs.readdirSync(dir, { withFileTypes: true })
      .some(e => e.isFile() && e.name.endsWith(".tex"));
  } catch {
    return false;
  }
}

// ── path normalisation ─────────────────────────────────────────────────

/** Convert a `file://` URI or plain path into a normalised absolute path. */
function normalizePath(raw: string | null): string | null {
  if (!raw) return null;
  try {
    let p = raw;
    if (p.startsWith("file://")) {
      p = decodeURIComponent(p.slice("file://".length));
    }
    // On Windows `file:///C:/…` decodes to `/C:/…` — strip the spurious
    // leading slash before the drive letter.
    if (process.platform === "win32" && /^\/[a-zA-Z]:[/\\]/.test(p)) {
      p = p.slice(1);
    }
    return path.normalize(p);
  } catch {
    return raw;
  }
}
