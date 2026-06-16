import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import {
  initWorkspaceMacros,
  updateFileMacros,
  getWorkspaceMacros,
  setSidecar,
  _resetForTesting,
} from "../src/preamble.js";
import type { SidecarHandle } from "../src/rust_sidecar.js";

// preamble.ts keeps module‑level state; reset it between tests.
beforeEach(() => _resetForTesting());

/** Build a throwaway project tree under the OS temp dir and return its root. */
function makeProject(layout: Record<string, string>): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "latex-test-"));
  for (const [rel, content] of Object.entries(layout)) {
    const full = path.join(root, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content, "utf-8");
  }
  return root;
}

function uri(p: string): string {
  return "file:///" + p.replace(/\\/g, "/").replace(/^\//, "");
}

test("preamble: macro from a sibling file at the project root is discovered", async () => {
  const root = makeProject({
    "macros.tex": "\\def\\O{\\Omega}\n",
    "content/ch1.tex": "text $\\O$ more\n",
  });
  try {
    const ch1 = path.join(root, "content", "ch1.tex");
    // Simulate Zed opening the file in a sub‑directory: didOpen fires with
    // the file's own location, not the true project root.
    updateFileMacros(uri(ch1), fs.readFileSync(ch1, "utf-8"));
    const macros = await getWorkspaceMacros();
    assert.deepEqual(macros.O, { body: "\\Omega", arity: 0 },
      "\\O should be discovered from sibling macros.tex");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("preamble: a wrong rootUri from onInitialize is widened on didOpen", async () => {
  const root = makeProject({
    "analysis.tex": "\\def\\O{\\Omega}\n",
    "content/functional.tex": "see $\\O$\n",
  });
  try {
    // Zed opened the `content/` sub‑directory, so rootUri points there.
    const wrongRoot = path.join(root, "content");
    initWorkspaceMacros(uri(wrongRoot));
    let macros = await getWorkspaceMacros();
    assert.equal(macros.O, undefined, "wrong root should not see analysis.tex");

    // Now didOpen the file — the inferred root widens to the real project root.
    const func = path.join(root, "content", "functional.tex");
    updateFileMacros(uri(func), fs.readFileSync(func, "utf-8"));
    macros = await getWorkspaceMacros();
    assert.deepEqual(macros.O, { body: "\\Omega", arity: 0 },
      "didOpen should widen the root and find \\O");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("preamble: re‑scan is not triggered when root stays the same", async () => {
  const root = makeProject({
    "a.tex": "\\def\\X{x}\n",
    "b.tex": "text\n",
  });
  try {
    const a = path.join(root, "a.tex");
    const b = path.join(root, "b.tex");
    initWorkspaceMacros(uri(root));
    updateFileMacros(uri(a), fs.readFileSync(a, "utf-8"));
    updateFileMacros(uri(b), fs.readFileSync(b, "utf-8"));
    const macros = await getWorkspaceMacros();
    assert.deepEqual(macros.X, { body: "x", arity: 0 });
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

// ── sidecar path: workspace_macros IPC should be consulted ────────────
//
// Regression: previously `getWorkspaceMacros` had a fast path that
// returned the (empty) in-process fileCache when sidecar was alive but
// the prime IPC hadn't completed yet.  Macros defined in files Zed
// never opened (e.g. preamble.tex) were lost.

function fakeSidecar(
  macros: Array<{ name: string; body: string; arity: number }>,
): SidecarHandle {
  return {
    workspace_macros: async () => ({ macros }),
    update_file: async () => ({ ok: true, parse_ms: 0, labels: [], macros: [] }),
    close_file: async () => ({ ok: true }),
    lookup: async () => ({ found: false }),
    cursor_context: async () => ({ kind: "none" as const }),
    doc_lookup: async () => ({ found: false }),
    ping: async () => ({ ok: true, uptime_ms: 0 }),
    shutdown: async () => {},
    isExited: () => false,
  };
}

test("preamble: sidecar workspace_macros are fetched when fileCache is empty", async () => {
  // No initWorkspaceMacros, no didOpen — fileCache stays empty.
  // The sidecar reports \R defined in a file Zed never opened.
  const sidecar = fakeSidecar([
    { name: "R", body: "\\mathbb{R}", arity: 0 },
  ]);
  setSidecar(sidecar);
  try {
    const macros = await getWorkspaceMacros();
    assert.deepEqual(
      macros.R,
      { body: "\\mathbb{R}", arity: 0 },
      "sidecar's workspace_macros must be used when fileCache is empty",
    );
  } finally {
    setSidecar(null);
  }
});

test("preamble: sidecar result is cached after first IPC", async () => {
  let calls = 0;
  const sidecar: SidecarHandle = {
    ...fakeSidecar([{ name: "R", body: "\\mathbb{R}", arity: 0 }]),
    workspace_macros: async () => {
      calls++;
      return { macros: [{ name: "R", body: "\\mathbb{R}", arity: 0 }] };
    },
  };
  setSidecar(sidecar);
  try {
    await getWorkspaceMacros();
    await getWorkspaceMacros();
    await getWorkspaceMacros();
    assert.equal(calls, 1, "subsequent calls must hit the in-memory cache");
  } finally {
    setSidecar(null);
  }
});

test("preamble: invalidate (via updateFileMacros) re-primes on next call", async () => {
  // After updateFileMacros -> invalidate -> cachedMacros=null, the next
  // getWorkspaceMacros must re-fetch from sidecar (so the user sees
  // fresh macros after editing preamble.tex externally).
  let calls = 0;
  const sidecar: SidecarHandle = {
    ...fakeSidecar([{ name: "R", body: "\\mathbb{R}", arity: 0 }]),
    workspace_macros: async () => {
      calls++;
      return { macros: [{ name: "R", body: "\\mathbb{R}", arity: 0 }] };
    },
    update_file: async () => ({ ok: true, parse_ms: 0, labels: [], macros: [] }),
  };
  setSidecar(sidecar);
  try {
    await getWorkspaceMacros();
    assert.equal(calls, 1);
    // Simulate didOpen: writes to fileCache, then invalidates.
    updateFileMacros(uri(path.join(os.tmpdir(), "main.tex")), "");
    assert.equal(calls, 1, "invalidate alone must not re-fetch");
    await getWorkspaceMacros();
    assert.equal(calls, 2, "next call after invalidate must re-fetch");
  } finally {
    setSidecar(null);
  }
});
