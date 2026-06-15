import { test } from "node:test";
import assert from "node:assert/strict";
import { docHoverFor } from "../src/doc_hover.js";
import type { SidecarHandle } from "../src/rust_sidecar.js";

function makeSidecar(
  result:
    | { found: false }
    | { found: true; entry: { kind: "package" | "command"; title: string; short: string; docs?: string } },
): SidecarHandle {
  return {
    async doc_lookup(_name: string) {
      return result;
    },
  } as unknown as SidecarHandle;
}

// ── package ───────────────────────────────────────────────────────────

test("found_renders_package_with_short", async () => {
  const out = await docHoverFor("amsmath", makeSidecar({
    found: true,
    entry: {
      kind: "package",
      title: "amsmath",
      short: "Core math extension for LaTeX.",
    },
  }));
  assert.ok(out);
  assert.match(out!.contents.value, /\*\*amsmath\*\* \(package\)/);
  assert.match(out!.contents.value, /Core math extension for LaTeX\./);
});

test("found_renders_package_with_docs_over_short", async () => {
  const out = await docHoverFor("amsmath", makeSidecar({
    found: true,
    entry: {
      kind: "package",
      title: "amsmath",
      short: "Short.",
      docs: "Longer body used when present.",
    },
  }));
  assert.ok(out);
  assert.match(out!.contents.value, /Longer body used when present\./);
  assert.doesNotMatch(out!.contents.value, /^Short\.$/m);
});

// ── command ───────────────────────────────────────────────────────────

test("found_renders_command", async () => {
  const out = await docHoverFor("textbf", makeSidecar({
    found: true,
    entry: {
      kind: "command",
      title: "textbf",
      short: "Bold text.",
    },
  }));
  assert.ok(out);
  assert.match(out!.contents.value, /\*\*textbf\*\* \(command\)/);
  assert.match(out!.contents.value, /Bold text\./);
});

// ── not found / error ─────────────────────────────────────────────────

test("not_found_returns_null", async () => {
  const out = await docHoverFor("not-a-real-name", makeSidecar({ found: false }));
  assert.equal(out, null);
});

test("sidecar_error_returns_null", async () => {
  const broken = {
    async doc_lookup() {
      throw new Error("sidecar dead");
    },
  } as unknown as SidecarHandle;
  const out = await docHoverFor("amsmath", broken);
  assert.equal(out, null);
});
