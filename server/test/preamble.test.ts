import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import { initWorkspaceMacros, updateFileMacros, getWorkspaceMacros, _resetForTesting } from "../src/preamble.js";

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

test("preamble: macro from a sibling file at the project root is discovered", () => {
  const root = makeProject({
    "macros.tex": "\\def\\O{\\Omega}\n",
    "content/ch1.tex": "text $\\O$ more\n",
  });
  try {
    const ch1 = path.join(root, "content", "ch1.tex");
    // Simulate Zed opening the file in a sub‑directory: didOpen fires with
    // the file's own location, not the true project root.
    updateFileMacros(uri(ch1), fs.readFileSync(ch1, "utf-8"));
    const macros = getWorkspaceMacros();
    assert.deepEqual(macros.O, { body: "\\Omega", arity: 0 },
      "\\O should be discovered from sibling macros.tex");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("preamble: a wrong rootUri from onInitialize is widened on didOpen", () => {
  const root = makeProject({
    "analysis.tex": "\\def\\O{\\Omega}\n",
    "content/functional.tex": "see $\\O$\n",
  });
  try {
    // Zed opened the `content/` sub‑directory, so rootUri points there.
    const wrongRoot = path.join(root, "content");
    initWorkspaceMacros(uri(wrongRoot));
    let macros = getWorkspaceMacros();
    assert.equal(macros.O, undefined, "wrong root should not see analysis.tex");

    // Now didOpen the file — the inferred root widens to the real project root.
    const func = path.join(root, "content", "functional.tex");
    updateFileMacros(uri(func), fs.readFileSync(func, "utf-8"));
    macros = getWorkspaceMacros();
    assert.deepEqual(macros.O, { body: "\\Omega", arity: 0 },
      "didOpen should widen the root and find \\O");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("preamble: re‑scan is not triggered when root stays the same", () => {
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
    const macros = getWorkspaceMacros();
    assert.deepEqual(macros.X, { body: "x", arity: 0 });
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
