//! Workspace‑wide macro auto‑discovery.
//!
//! The LSP discovers `\newcommand`, `\def`, … definitions from every `*.tex`
//! file reachable from the project root and merges them into one macro map.
//! The project root is inferred from each opened file by walking upward until
//! a directory no longer contains `.tex` files, so the discovery works even
//! when Zed's `rootUri` points at a sub‑directory of the real project.
//!
//! Whenever a wider root is discovered (e.g. a file is opened whose inferred
//! root is an ancestor of the previously scanned root), the workspace is
//! re‑scanned so macros defined in sibling trees become visible.  Per‑file
//! edits on `didOpen`/`didChange` always update that file's entry.

import * as fs from "node:fs";
import * as path from "node:path";
import { extractMacros, type MacroMap } from "./macros.js";

// ── state ──────────────────────────────────────────────────────────────

/** Per‑file cache: normalised absolute path → macro map. */
const fileCache = new Map<string, MacroMap>();
/** Directory currently scanned as the workspace root (null = never scanned). */
let workspaceRoot: string | null = null;

/** Return the merged macro map from all discovered workspace files. */
export function getWorkspaceMacros(): MacroMap {
  const merged: MacroMap = {};
  for (const m of fileCache.values()) Object.assign(merged, m);
  return merged;
}

/** Reset all cached state.  Test‑only hook. */
export function _resetForTesting(): void {
  fileCache.clear();
  workspaceRoot = null;
}

/**
 * Seed the workspace root from Zed's `rootUri` (called once on `onInitialize`).
 * This is only a hint — the root is widened later if an opened file implies a
 * larger project.  No‑op when `rootUri` is null/unparseable.
 */
export function initWorkspaceMacros(rootUri: string | null): void {
  const root = normalizePath(rootUri);
  if (root) ensureScanned(root);
}

/**
 * Update the macro cache for one file (called on every `didOpen`/`didChange`).
 * First widens the workspace root if the file lives in a project larger than
 * the currently scanned root, then (re)writes this file's own macros.
 */
export function updateFileMacros(uri: string, text: string): void {
  const filePath = normalizePath(uri);
  if (!filePath) return;
  // Infer the project root from this file and re‑scan if it is wider than
  // what we have so far (covers the case where Zed's rootUri was a sub‑dir).
  const inferred = inferProjectRoot(filePath);
  if (inferred) ensureScanned(inferred);
  fileCache.set(filePath, extractMacros(text));
}

// ── internal scan ──────────────────────────────────────────────────────

/** Scan `root` only if it is strictly wider than (an ancestor of) the
 *  currently scanned root.  Re‑scanning re‑reads every `*.tex` file under
 *  the new root, so previously unseen sibling trees become visible. */
function ensureScanned(root: string): void {
  if (workspaceRoot && (root === workspaceRoot || isAncestor(workspaceRoot, root))) {
    return; // already covered (workspaceRoot is root or a sub‑dir of root)
  }
  workspaceRoot = root;
  fileCache.clear();
  for (const abs of collectTexFiles(root)) {
    try {
      fileCache.set(abs, extractMacros(fs.readFileSync(abs, "utf-8")));
    } catch {
      // File disappeared / permission denied — skip silently.
    }
  }
}

// ── root inference ─────────────────────────────────────────────────────

/** Walk upward from `filePath`'s directory to the top‑most directory that
 *  still contains `.tex` files directly inside it.  Returns that directory,
 *  or null if the file's own directory has none (and no parent does). */
function inferProjectRoot(filePath: string): string | null {
  let dir = path.dirname(filePath);
  if (!hasTexFiles(dir)) return null;
  for (;;) {
    const parent = path.dirname(dir);
    if (parent === dir || !hasTexFiles(parent)) return dir;
    dir = parent;
  }
}

/** True when `maybeAncestor` is `dir` itself or a parent directory of it. */
function isAncestor(maybeAncestor: string, dir: string): boolean {
  if (maybeAncestor === dir) return true;
  const a = maybeAncestor + path.sep;
  return dir.startsWith(a);
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
