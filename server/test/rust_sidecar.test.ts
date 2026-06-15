import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import {
  resolveSidecarPath,
  startSidecar,
  type SidecarHandle,
} from "../src/rust_sidecar.js";

// ── resolveSidecarPath: missing binary ────────────────────────────────

test("resolveSidecarPath returns null when explicit path is bogus", () => {
  const r = resolveSidecarPath(path.join(os.tmpdir(), "definitely-not-a-binary-xyzzy"));
  assert.equal(r, null);
});

test("resolveSidecarPath returns null when env var points to a missing file", () => {
  const orig = process.env.LATEX_INDEX_PATH;
  try {
    process.env.LATEX_INDEX_PATH = path.join(os.tmpdir(), "nope-latex-index-missing");
    const r = resolveSidecarPath();
    // Might still find a binary on PATH or in cargo target, but not from
    // the env var.  We just check that the env path is NOT the returned
    // value (because it doesn't exist on disk).
    if (r !== null) {
      assert.notEqual(r, path.resolve(process.env.LATEX_INDEX_PATH!));
    }
  } finally {
    if (orig === undefined) delete process.env.LATEX_INDEX_PATH;
    else process.env.LATEX_INDEX_PATH = orig;
  }
});

test("resolveSidecarPath returns the explicit path when it exists", () => {
  // Use a real, existing file as a stand-in.
  const existing = fileURLToPath(import.meta.url); // this test file
  const r = resolveSidecarPath(existing);
  assert.equal(r, path.resolve(existing));
});

// ── startSidecar: graceful null when binary not found ────────────────

test("startSidecar returns null when binPath is missing", async () => {
  const r = await startSidecar({
    binPath: path.join(os.tmpdir(), "no-such-binary-zzz"),
    rootUri: null,
  });
  assert.equal(r, null);
});

test("startSidecar returns null when nothing is resolvable (forced)", async () => {
  // Force a clean resolve by passing an explicit bad path; this short-
  // circuits PATH / cargo lookups via the early `if (explicit && ...)` branch.
  const r = await startSidecar({
    binPath: path.join(os.tmpdir(), "missing-1") + path.sep + "missing-2",
    rootUri: null,
  });
  assert.equal(r, null);
});

// ── SidecarHandle: NDJSON pairing with a mock binary ─────────────────

/**
 * Build a tiny Node script that speaks the NDJSON protocol the sidecar
 * expects.  The trick: we use `process.execPath` (the running node.exe)
 * as the binary and inject the mock via `NODE_OPTIONS=--require=<path>`,
 * so spawn(bin, ...) always works (a real .exe on every platform).
 *
 * The mock records every (id, method) it received into a log file so
 * tests can assert pairing.
 */
function writeMockSidecar(dir: string): { bin: string; log: string; env: NodeJS.ProcessEnv } {
  const log = path.join(dir, "requests.log");
  const scriptPath = path.join(dir, "mock-sidecar.cjs");
  const script = `
const fs = require("fs");
const logPath = ${JSON.stringify(log)};
function respond(id, result) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id, result }) + "\\n");
}
const rl = require("readline").createInterface({ input: process.stdin });
rl.on("line", (line) => {
  if (!line.trim()) return;
  let msg;
  try { msg = JSON.parse(line); } catch { return; }
  fs.appendFileSync(logPath, JSON.stringify({ id: msg.id, method: msg.method }) + "\\n");
  if (msg.method === "initialize") {
    respond(msg.id, { ok: true, capabilities: { kinds: ["cite","ref","math"] }, version: 1 });
  } else if (msg.method === "ping") {
    respond(msg.id, { ok: true, uptime_ms: 1 });
  } else if (msg.method === "workspace_macros") {
    respond(msg.id, { macros: [] });
  } else if (msg.method === "lookup") {
    respond(msg.id, { found: false });
  } else if (msg.method === "cursor_context") {
    respond(msg.id, { kind: "none" });
  } else if (msg.method === "update_file") {
    respond(msg.id, { ok: true, parse_ms: 1, labels: [], macros: [] });
  } else if (msg.method === "close_file") {
    respond(msg.id, { ok: true });
  } else {
    process.stdout.write(JSON.stringify({ jsonrpc:"2.0", id: msg.id, error: { code: -32601, message: "unknown" }}) + "\\n");
  }
});
process.on("SIGTERM", () => process.exit(0));
process.on("SIGINT", () => process.exit(0));
`;
  fs.writeFileSync(scriptPath, script, "utf8");
  const env = { NODE_OPTIONS: `--require=${scriptPath}` };
  return { bin: process.execPath, log, env };
}

async function withTempDir<T>(fn: (dir: string) => Promise<T>): Promise<T> {
  const dir = await fs.promises.mkdtemp(path.join(os.tmpdir(), "sidecar-test-"));
  try {
    return await fn(dir);
  } finally {
    await fs.promises.rm(dir, { recursive: true, force: true });
  }
}

function readIds(log: string): number[] {
  if (!fs.existsSync(log)) return [];
  return fs
    .readFileSync(log, "utf8")
    .split("\n")
    .filter(Boolean)
    .map((l) => JSON.parse(l).id as number);
}

test("startSidecar spawns the mock binary and pairs request ids", async () => {
  await withTempDir(async (dir) => {
    const { bin, log, env } = writeMockSidecar(dir);
    const sidecar = (await startSidecar({ binPath: bin, rootUri: null, env })) as SidecarHandle | null;
    assert.ok(sidecar, "expected the mock sidecar to spawn");

    // Each call below should map to a unique id seen by the mock.
    await sidecar!.ping();
    await sidecar!.ping();
    await sidecar!.workspace_macros();
    const lc = await sidecar!.lookup("einstein1905", "cite");
    const ctx = await sidecar!.cursor_context("file:///x.tex", 42);
    void lc; void ctx;

    const ids = readIds(log);
    // initialize, ping, ping, workspace_macros, lookup, cursor_context
    assert.equal(ids.length, 6);
    // All ids are unique and start at 1.
    const sorted = [...ids].sort((a, b) => a - b);
    assert.deepEqual(sorted, [1, 2, 3, 4, 5, 6]);
    // Methods are recorded in order.
    const lines = fs.readFileSync(log, "utf8").split("\n").filter(Boolean);
    const methods = lines.map((l) => JSON.parse(l).method as string);
    assert.deepEqual(methods, [
      "initialize",
      "ping",
      "ping",
      "workspace_macros",
      "lookup",
      "cursor_context",
    ]);

    await sidecar!.shutdown();
  });
});

test("startSidecar returns null when spawn fails (non-existent script via NODE_OPTIONS)", async () => {
  // Pointing NODE_OPTIONS at a missing require target causes node to bail
  // before reading stdin -> the sidecar never initialises -> null.
  await withTempDir(async (dir) => {
    const r = await startSidecar({
      binPath: process.execPath,
      rootUri: null,
      env: { NODE_OPTIONS: "--require=" + path.join(dir, "definitely-missing.cjs") },
    });
    assert.equal(r, null);
  });
});

test("startSidecar rejects pending requests if the sidecar dies", async () => {
  await withTempDir(async (dir) => {
    const { bin, env } = writeMockSidecar(dir);
    const sidecar = (await startSidecar({ binPath: bin, rootUri: null, env })) as SidecarHandle | null;
    assert.ok(sidecar);

    // After shutdown, subsequent requests must reject.
    await sidecar!.shutdown();
    await assert.rejects(sidecar!.ping(), /not running/);
  });
});

// ── Optional smoke test against the real binary ──────────────────────

const cargoCandidates = [
  // ext-root (parent of server/) and the project's relative path
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "latex-index", "target", "release",
    process.platform === "win32" ? "latex-index.exe" : "latex-index"),
  path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "latex-index", "target", "debug",
    process.platform === "win32" ? "latex-index.exe" : "latex-index"),
];

const realBin = cargoCandidates.find((p) => fs.existsSync(p));

test("smoke: real latex-index binary (initialize, ping, shutdown)", { skip: !realBin }, async (t) => {
  if (!realBin) {
    t.skip("no real binary at " + cargoCandidates.join(" or "));
    return;
  }
  const sidecar = await startSidecar({ binPath: realBin, rootUri: null });
  assert.ok(sidecar, "real binary should spawn");
  // The real binary's `initialize` response shape is
  // `{ok, capabilities, version}`.  We don't read the return value, but
  // the call returning at all means initialize was paired correctly.
  const pong = await sidecar!.ping();
  assert.equal(pong.ok, true);
  await sidecar!.shutdown();
});

// ── Sanity: ensure spawn module loads (no regressions in imports) ────

test("resolveSidecarPath with a tmpdir-bogus PATH does not throw", () => {
  // Should never throw, even on Windows or when PATH is empty.
  const origPath = process.env.PATH;
  try {
    process.env.PATH = "";
    const r = resolveSidecarPath();
    // Might be null or might find the cargo target — either way, no throw.
    assert.ok(r === null || typeof r === "string");
  } finally {
    process.env.PATH = origPath;
  }
});
