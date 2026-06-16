//! Node-side client for the `latex-index` Rust sidecar.
//!
//! Spawns the binary as a child process, talks NDJSON over stdio, pairs
//! request/response by `id`.  If the binary is missing or fails to spawn,
//! `startSidecar` returns `null` and the caller falls back to the legacy
//! in-process extractor (`preamble.ts`).
//!
//! Public API (matches plan-ref-cite-hover.md Section 7):
//!   const sidecar = await startSidecar({ binPath?, rootUri?, env? });
//!   if (sidecar) {
//!     await sidecar.update_file(uri, text);
//!     await sidecar.close_file(uri);
//!     const r = await sidecar.lookup("einstein1905", "cite");
//!     const c = await sidecar.cursor_context(uri, offset);
//!     const m = await sidecar.workspace_macros();
//!     await sidecar.ping();
//!     await sidecar.shutdown();
//!   }
//!
//! Bounded in-flight queue (max 256): if exceeded, the oldest pending
//! request is rejected with `{ ok: false, error: "overflow" }`.

import { spawn, ChildProcess, SpawnOptions } from "node:child_process";
import { createInterface, Interface as RLInterface } from "node:readline";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import type {
  RpcResponseOk,
  RpcResponseErr,
  CursorContext,
  CiteLookupResult,
  RefLookupResult,
  WorkspaceMacrosResult,
  InitializeResult,
  UpdateFileResult,
  PingResult,
  DocLookupResult,
} from "./rpc_types.js";

// ── configuration ──────────────────────────────────────────────────────

export interface SidecarLaunchOpts {
  /** Override the binary path (else auto-resolve). */
  binPath?: string | null;
  /** LSP workspace root URI; passed to the sidecar as `initialize.rootUri`. */
  rootUri: string | null;
  /** Optional env additions. */
  env?: NodeJS.ProcessEnv;
  /** Hard cap on in-flight requests; default 256. */
  maxInFlight?: number;
}

const PROTOCOL_VERSION = 1;

// ── resolveSidecarPath ─────────────────────────────────────────────────

/**
 * Locate the `latex-index` binary.  Order:
 *   1. `binPath` argument (explicit override)
 *   2. `LATEX_INDEX_PATH` env var
 *   3. `<ext-dir>/latex-index/target/{release,debug}/latex-index{,.exe}`
 *      (cargo build output, relative to the running server.js)
 *   4. `latex-index` / `latex-index.exe` on PATH
 *
 * Returns the absolute path, or `null` if nothing matched.
 */
export function resolveSidecarPath(explicit?: string | null): string | null {
  if (explicit && fs.existsSync(explicit)) {
    return path.resolve(explicit);
  }
  const fromEnv = process.env.LATEX_INDEX_PATH;
  if (fromEnv && fs.existsSync(fromEnv)) {
    return path.resolve(fromEnv);
  }
  // Walk up from the running `server.js` to find `latex-index/target/{release,debug}/`.
  // server.js lives at <ext-root>/server/out/src/server.js
  // We need <ext-root>/latex-index/target/{release,debug}/latex-index{,.exe}
  const candidates = candidateCargoOutputs();
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  // Last resort: look on PATH.
  const onPath = whichOnPath(
    process.platform === "win32" ? "latex-index.exe" : "latex-index",
  );
  return onPath;
}

function candidateCargoOutputs(): string[] {
  // server.js may be bundled at <ext-root>/server/out/src/server.js, or run
  // via tsx from <ext-root>/server/src/server.ts.  Resolve both.
  const ext = path.dirname(findExtRoot());
  const exe = process.platform === "win32" ? "latex-index.exe" : "latex-index";
  const targets = ["release", "debug"];
  const out: string[] = [];
  for (const t of targets) {
    out.push(path.join(ext, "latex-index", "target", t, exe));
  }
  return out;
}

function findExtRoot(): string {
  // If we are running compiled JS, __dirname is the directory of server.js.
  // The Rust extension is mounted at <ext-root>/, with server/ as a subdir.
  // We walk up: out/src -> out -> server -> <ext-root>.
  // If running via tsx from src/, we go up two levels: src -> server -> <ext-root>.
  const here = path.dirname(fileURLToPath(import.meta.url));
  // Try going up 3 levels first (compiled layout), then 2 (dev layout).
  for (const n of [3, 2, 1]) {
    let p = here;
    for (let i = 0; i < n; i++) p = path.dirname(p);
    // Heuristic: must contain Cargo.toml OR be parent of server/ AND latex-index/.
    if (
      fs.existsSync(path.join(p, "extension.toml")) ||
      (fs.existsSync(path.join(p, "server")) &&
        fs.existsSync(path.join(p, "latex-index")))
    ) {
      return p;
    }
  }
  return here;
}

function whichOnPath(bin: string): string | null {
  const sep = process.platform === "win32" ? ";" : ":";
  const dirs = (process.env.PATH || "").split(sep);
  const pathext = (process.env.PATHEXT || ".EXE;.BAT;.CMD").split(";");
  for (const d of dirs) {
    if (!d) continue;
    const candidates =
      process.platform === "win32"
        ? pathext.map((ext) => path.join(d, bin + ext))
        : [path.join(d, bin)];
    for (const c of candidates) {
      try {
        const st = fs.statSync(c);
        if (st.isFile()) return c;
      } catch {
        // ignore
      }
    }
  }
  return null;
}

// ── pending request bookkeeping ────────────────────────────────────────

interface Pending {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
  method: string;
  enqueuedAt: number;
}

class SidecarHandle {
  private child: ChildProcess;
  private rl: RLInterface;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private inFlight = 0;
  private maxInFlight: number;
  private shuttingDown = false;
  private buffer: string = "";
  private alive = true;
  /// Set to true when the child process has actually exited.  Differs
  /// from `alive = false` in that a single per-request JSON-RPC error
  /// does NOT flip this — only the process-level `exit` event does.
  /// Callers (e.g. `preamble.ts`) use this to decide whether to retry
  /// the sidecar or permanently give up.
  private exited = false;

  private constructor(
    child: ChildProcess,
    rl: RLInterface,
    maxInFlight: number,
  ) {
    this.child = child;
    this.rl = rl;
    this.maxInFlight = maxInFlight;
  }

  static async spawn(
    binPath: string,
    rootUri: string | null,
    opts: SidecarLaunchOpts,
  ): Promise<SidecarHandle | null> {
    const spawnOpts: SpawnOptions = {
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, ...(opts.env || {}) },
      windowsHide: true,
    };
    let child: ChildProcess;
    try {
      child = spawn(binPath, [], spawnOpts);
    } catch (e) {
      return null;
    }
    child.on("error", () => {
      // EPIPE / spawn failure — marked dead below.
    });
    const handle = new SidecarHandle(
      child,
      createInterface({ input: child.stdout! }),
      opts.maxInFlight ?? 256,
    );
    // Surface stderr to LSP logs once.
    let stderrLogged = false;
    child.stderr?.on("data", (chunk: Buffer) => {
      if (!stderrLogged) {
        stderrLogged = true;
        process.stderr.write(`[latex-index] ${chunk.toString()}`);
      }
    });
    child.on("exit", (code, signal) => {
      handle.alive = false;
      handle.exited = true;
      const reason = `sidecar exited (code=${code} signal=${signal})`;
      for (const p of handle.pending.values()) {
        p.reject(new Error(reason));
      }
      handle.pending.clear();
    });
    handle.rl.on("line", (line) => handle.onLine(line));
    // Initialise.
    try {
      await handle.initialize(rootUri);
    } catch (e) {
      try {
        child.kill();
      } catch {
        /* ignore */
      }
      return null;
    }
    return handle;
  }

  private async initialize(rootUri: string | null): Promise<InitializeResult> {
    return (await this.request("initialize", {
      rootUri,
      version: PROTOCOL_VERSION,
    })) as InitializeResult;
  }

  // ── public RPC methods ───────────────────────────────────────────────

  async update_file(uri: string, text: string): Promise<UpdateFileResult> {
    return (await this.request("update_file", { uri, text })) as UpdateFileResult;
  }

  async close_file(uri: string): Promise<{ ok: boolean }> {
    return (await this.request("close_file", { uri })) as { ok: boolean };
  }

  async lookup(key: string, kind: "cite"): Promise<CiteLookupResult>;
  async lookup(key: string, kind: "ref"): Promise<RefLookupResult>;
  async lookup(
    key: string,
    kind: "cite" | "ref",
  ): Promise<CiteLookupResult | RefLookupResult> {
    return (await this.request("lookup", { key, kind })) as
      | CiteLookupResult
      | RefLookupResult;
  }

  async cursor_context(uri: string, offset: number): Promise<CursorContext> {
    return (await this.request("cursor_context", { uri, offset })) as CursorContext;
  }

  /** `doc_lookup` (Phase 2 §4.9) — bundled package/command dictionary. */
  async doc_lookup(name: string): Promise<DocLookupResult> {
    return (await this.request("doc_lookup", { name })) as DocLookupResult;
  }

  async workspace_macros(): Promise<WorkspaceMacrosResult> {
    return (await this.request("workspace_macros", {})) as WorkspaceMacrosResult;
  }

  async ping(): Promise<PingResult> {
    return (await this.request("ping", {})) as PingResult;
  }

  /**
   * True once the child process has exited.  Differs from "the most
   * recent request failed": a single per-method JSON-RPC error reply
   * does not flip this.  Use this to decide whether a transient
   * rejection should be retried or whether the sidecar is permanently
   * dead and the in-process fallback should take over.
   */
  isExited(): boolean {
    return this.exited;
  }

  async shutdown(): Promise<void> {
    if (this.shuttingDown || !this.alive) return;
    this.shuttingDown = true;
    try {
      this.child.stdin?.end();
    } catch {
      /* ignore */
    }
    // Give it 250ms to flush, then SIGTERM.
    await new Promise((r) => setTimeout(r, 250));
    if (this.alive) {
      try {
        this.child.kill();
      } catch {
        /* ignore */
      }
    }
  }

  // ── internals ────────────────────────────────────────────────────────

  private async request(method: string, params: unknown): Promise<unknown> {
    if (!this.alive) {
      throw new Error("sidecar not running");
    }
    // Bounded queue: if exceeded, reject the oldest.
    if (this.inFlight >= this.maxInFlight && this.pending.size > 0) {
      const oldestId = this.pending.keys().next().value as number;
      const oldest = this.pending.get(oldestId);
      if (oldest) {
        oldest.reject(new Error("overflow"));
        this.pending.delete(oldestId);
        this.inFlight--;
      }
    }
    const id = this.nextId++;
    return await new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject, method, enqueuedAt: Date.now() });
      this.inFlight++;
      const line = JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n";
      try {
        this.child.stdin?.write(line);
      } catch (e) {
        this.pending.delete(id);
        this.inFlight--;
        reject(e instanceof Error ? e : new Error(String(e)));
      }
    });
  }

  private onLine(line: string): void {
    const trimmed = line.trim();
    if (!trimmed) return;
    let msg: RpcResponseOk | RpcResponseErr;
    try {
      msg = JSON.parse(trimmed);
    } catch (e) {
      // Skip malformed lines.
      return;
    }
    if (typeof msg.id !== "number") return;
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    this.inFlight = Math.max(0, this.inFlight - 1);
    if ("error" in msg) {
      p.reject(new Error(`${msg.error.message} (code=${msg.error.code})`));
    } else {
      p.resolve(msg.result);
    }
  }
}

// ── public entry point ─────────────────────────────────────────────────

/**
 * Try to spawn the sidecar.  Returns the handle on success, or `null` if
 * the binary is missing / fails to spawn.  Callers MUST check for `null`
 * and fall back to the in-process extractor.
 */
export async function startSidecar(
  opts: SidecarLaunchOpts,
): Promise<SidecarHandle | null> {
  const binPath = resolveSidecarPath(opts.binPath);
  console.error(`[latex-preview] startSidecar: binPath=${binPath ?? "null"} rootUri=${opts.rootUri ?? "null"}`);
  if (!binPath) return null;
  try {
    const h = await SidecarHandle.spawn(binPath, opts.rootUri, opts);
    console.error(`[latex-preview] startSidecar.spawn returned ${h ? "handle" : "null"}`);
    return h;
  } catch (e) {
    console.error(`[latex-preview] startSidecar.spawn threw: ${e}`);
    throw e;
  }
}

export type { SidecarHandle };
