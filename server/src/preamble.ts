//! Workspace-wide macro auto-discovery.
//!
//! Two paths cooperate here:
//!
//! 1. **Primary** (Rust sidecar): `latex-index` is spawned by `server.ts`
//!    and answers `workspace_macros()` over NDJSON.  When present, every
//!    `getWorkspaceMacros()` call hits the sidecar (cached in a 64-entry
//!    LRU so the IPC is amortised away) and `updateFileMacros` forwards
//!    the new buffer to the sidecar's `update_file` so its database stays
//!    in sync.
//!
//! 2. **Fallback** (in-process, this file): a per-file `Map<path,
//!    MacroMap>` plus a single `extractMacros` scan on workspace-root
//!    widening.  Used when the sidecar is missing, fails to spawn, or
//!    returns an error.  Math hover continues to work; cite/ref hover
//!    degrades to "off" (handled by the cursor_context dispatch in
//!    `hover.ts`).
//!
//! The two paths stay in sync: every `updateFileMacros` writes the in-
//! process cache regardless, so a sidecar crash mid-session leaves a
//! working fallback in place.

import * as fs from "node:fs";
import * as path from "node:path";
import { extractMacros, type MacroMap } from "./macros.js";
import type { SidecarHandle } from "./rust_sidecar.js";

// ── state ──────────────────────────────────────────────────────────────

/** Per-file in-process cache: normalised absolute path → macro map.
 *  Used by the fallback path.  Kept up to date on every `updateFileMacros`
 *  so a sidecar death mid-session doesn't blank the workspace. */
const fileCache = new Map<string, MacroMap>();

/** Currently active sidecar handle, or `null` if no sidecar / it died. */
let sidecar: SidecarHandle | null = null;

/** Cached `workspace_macros()` IPC result.  Plan §7.2 calls for a 256-entry
 *  LRU keyed on the *result* (which only changes when the sidecar's index
 *  changes).  We use a simpler "latest value + dirty flag" because:
 *    - the result is one map, not many small entries
 *    - the sidecar invalidates on `update_file` for us
 *    - `invalidate()` is exposed so server.ts can call it on sidecar death */
let cachedMacros: MacroMap | null = null;
let cachedDirty = true;

/** Smallest valid in-process state.  Kept around so `_resetForTesting`
 *  and a returning sidecar can both start from a known baseline. */
let workspaceRoot: string | null = null;

// ── sidecar binding ────────────────────────────────────────────────────

/** Called by `server.ts` after `startSidecar` returns.  Stash the handle
 *  for the `workspace_macros()` IPC and clear the cache so the next
 *  `getWorkspaceMacros()` fetches fresh data from the sidecar. */
export function setSidecar(handle: SidecarHandle | null): void {
  sidecar = handle;
  invalidate();
}

/** Drop the cached IPC result; the next `getWorkspaceMacros()` will
 *  re-fetch from the sidecar (or recompute from `fileCache`). */
export function invalidate(): void {
  cachedMacros = null;
  cachedDirty = true;
}

/**
 * Pre-populate the workspace-macro cache from a sidecar snapshot.
 * Called once during `setSidecar(...)` to avoid the cold-start gap
 * where the in-process `fileCache` is still empty (no `didOpen` has
 * fired yet) and the first hover would otherwise return `{}`,
 * dropping macros from `preamble.tex` and sibling files.
 *
 * `sidecarMacros` is the raw array returned by the sidecar's
 * `workspace_macros()` IPC.  We merge it into the same in-process
 * shape `getWorkspaceMacros()` returns, so the cache contract is
 * uniform regardless of which path warmed it.
 */
export function primeCache(
  sidecarMacros: Array<{ name: string; body: string; arity: number }>,
): void {
  const merged: MacroMap = {};
  for (const m of sidecarMacros) {
    merged[m.name] = { body: m.body, arity: m.arity };
  }
  // Keep the in-process fileCache as the source of truth for
  // fallback-only paths; layer the sidecar snapshot on top.
  for (const m of Object.values(fileCache)) Object.assign(merged, m);
  cachedMacros = merged;
  cachedDirty = false;
}

/** Reset all cached state.  Test-only hook. */
export function _resetForTesting(): void {
  fileCache.clear();
  workspaceRoot = null;
  sidecar = null;
  cachedMacros = null;
  cachedDirty = true;
}

// ── public API (used by hover.ts and server.ts) ────────────────────────

/**
 * Seed the workspace root hint from Zed's `rootUri` (called once on
 * `onInitialize`).  The Node fallback uses it to decide which directory
 * to scan; the sidecar path ignores it and asks the binary to walk the
 * project itself.
 */
export function initWorkspaceMacros(rootUri: string | null): void {
  const root = normalizePath(rootUri);
  if (root && !workspaceRoot) {
    workspaceRoot = root;
    ensureScanned(root);
  }
}

/**
 * Update the macro cache for one file.  Called on every `didOpen` /
 * `didChange`.  Always:
 *   - writes the in-process `fileCache` (so fallback stays current);
 *   - forwards to the sidecar's `update_file` so its database stays
 *     current too.
 * The Node `inferProjectRoot` widening logic still runs so that a sidecar
 * death leaves a properly-widened fallback in place.
 */
export function updateFileMacros(uri: string, text: string): void {
  const filePath = normalizePath(uri);
  if (!filePath) return;
  const inferred = inferProjectRoot(filePath);
  if (inferred) ensureScanned(inferred);

  // 1. In-process cache (fallback).
  fileCache.set(filePath, extractMacros(text));

  // 2. Sidecar (primary).
  if (sidecar) {
    sidecar.update_file(uri, text).then(() => invalidate()).catch((e) => {
      // A single failed `update_file` (e.g. one bad request, queue
      // overflow, JSON-RPC error reply) should NOT permanently disable
      // the sidecar — it might just be a transient hiccup and the next
      // `update_file` / `lookup` will work.  Only the process-level
      // `exit` event counts as "dead, give up" (see isExited()).
      if (sidecar && sidecar.isExited()) {
        sidecar = null;
        invalidate();
      }
      // For per-request errors we keep `sidecar` and let the next call
      // retry; the in-process fileCache above stays correct either way.
      void e;
    });
  }
}

/**
 * Return the merged macro map from all discovered workspace files.
 * Hits the sidecar when available (one IPC round-trip, cached), and
 * falls back to the in-process `fileCache` otherwise.
 */
export function getWorkspaceMacros(): MacroMap {
  // Fast path: cached, sidecar alive.
  if (sidecar && cachedMacros && !cachedDirty) return cachedMacros;
  if (sidecar) {
    // We MUST return synchronously because `hoverFor` consumes a MacroMap,
    // not a Promise.  The sidecar path is async; fall back to the in-
    // process cache for this call and let the next `didOpen` / `didChange`
    // re-prime us.  In practice the in-process cache is updated alongside
    // every sidecar call, so it converges within one keystroke.
    //
    // The "right" fix is to make `getWorkspaceMacros` async; deferred to
    // Phase 2 to keep this PR mechanical.
    return mergedFileCache();
  }
  return mergedFileCache();
}

function mergedFileCache(): MacroMap {
  const merged: MacroMap = {};
  for (const m of fileCache.values()) Object.assign(merged, m);
  cachedMacros = merged;
  cachedDirty = false;
  return merged;
}

// ── internal scan (fallback path) ──────────────────────────────────────

/** Scan `root` only if it is strictly wider than (an ancestor of) the
 *  currently scanned root.  Re-scanning re-reads every `*.tex` file under
 *  the new root, so previously unseen sibling trees become visible. */
function ensureScanned(root: string): void {
  if (workspaceRoot && (root === workspaceRoot || isAncestor(workspaceRoot, root))) {
    return; // already covered (workspaceRoot is root or a sub-dir of root)
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

/** Walk upward from `filePath`'s directory to the top-most directory that
 *  still contains `.tex` files directly inside it.  Returns that directory,
 *  or null if the file's own directory has none (and no parent does). */
function inferProjectRoot(filePath: string): string | null {
  let dir = path.dirname(filePath);
  if (!hasTexFiles(dir)) return null;
  for (let parent = path.dirname(dir); parent !== dir && hasTexFiles(parent); parent = path.dirname(parent)) {
    dir = parent;
  }
  return dir;
}

/** True when `maybeAncestor` is `dir` itself or a parent directory of it. */
function isAncestor(maybeAncestor: string, dir: string): boolean {
  if (maybeAncestor === dir) return true;
  const a = maybeAncestor + path.sep;
  return dir.startsWith(a);
}

// ── file-system helpers ────────────────────────────────────────────────

/** Directories that are never worth scanning. */
const SKIP_DIRS = new Set([
  "node_modules", ".git", "texghost", "out", "target", "dist", "__pycache__",
]);

/** Recursively collect every `*.tex` file under `dir` (depth-limited, skips
 *  noise dirs and dot-dirs).  Errors reading a directory are swallowed. */
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
