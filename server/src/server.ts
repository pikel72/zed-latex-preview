//! LSP server entry point.
//!
//! Wires up the `vscode‑languageserver` transport to our internal modules:
//!
//! ```text
//!                    ┌────────────────┐
//! textDocument/      │   server.ts    │
//! didOpen ──────────→│                │
//! didChange ────────→│  preamble.ts   │──── initWorkspaceMacros()
//! didClose ─────────→│                │──── updateFileMacros()
//!                    └────────────────┘
//! textDocument/hover ─→ hover.ts ──── HoverResult
//! ```
//!
//! The server responds to `textDocument/hover` requests only.  It does not
//! register completions, diagnostics, or any other LSP capability.
//!
//! ## Sidecar split (Phase 1)
//!
//! On `onInitialize` the Node LSP tries to spawn the `latex-index` Rust
//! sidecar (`rust_sidecar.ts`).  If that succeeds, every `didOpen` and
//! `didChangeContent` is forwarded to the sidecar, and `hoverFor` first
//! asks the sidecar `cursor_context` to decide between cite / ref / math.
//! If the sidecar is missing or fails to spawn we fall back to the
//! original in-process extractor (`preamble.ts`) — math hover is
//! unchanged.  See `docs/plan-ref-cite-hover.md` Section 2 for the
//! rationale.

import { createConnection, ProposedFeatures, TextDocuments, TextDocumentSyncKind } from "vscode-languageserver/node.js";
import { TextDocument } from "vscode-languageserver-textdocument";
import { configFromInit } from "./config.js";
import { hoverFor } from "./hover.js";
import { initWorkspaceMacros, updateFileMacros, setSidecar, primeCache } from "./preamble.js";
import { invalidateScannerCache } from "./scanner.js";
import { startSidecar, type SidecarHandle } from "./rust_sidecar.js";

const connection = createConnection(ProposedFeatures.all);
const docs = new TextDocuments(TextDocument);

let cfg = configFromInit(undefined);
let sidecar: SidecarHandle | null = null;
let sidecarWarned = false;

connection.onInitialize(async (params) => {
  cfg = configFromInit(params.initializationOptions);
  // Auto‑discover \def, \newcommand etc. from every .tex file in the
  // workspace.  Subsequent didOpen/didChange calls keep per‑file caches
  // up to date.
  initWorkspaceMacros(params.rootUri ?? null);

  // Spawn the Rust sidecar if the user hasn't disabled it.  Best-effort:
  // a missing binary logs once and we fall back to the in-process path.
  if (cfg.enabledSidecar) {
    try {
      sidecar = await startSidecar({
        binPath: cfg.sidecarPath,
        rootUri: params.rootUri ?? null,
      });
      setSidecar(sidecar);
      // Cold-start prime: fetch the sidecar's workspace-macros snapshot
      // eagerly so the first hover (which may fire before any didOpen)
      // already has macros from preamble.tex / sibling files.  See
      // preamble.ts::primeCache for the cold-start gap this closes.
      if (sidecar) {
        sidecar.workspace_macros().then((r) => primeCache(r.macros)).catch(() => {
          // Failure here is non-fatal: getWorkspaceMacros will fall back
          // to the in-process fileCache for the first few hovers until
          // the next didOpen / didChange re-primes us.
        });
      }
      if (!sidecar && !sidecarWarned) {
        sidecarWarned = true;
        connection.console.warn(
          "latex-index sidecar not found; cite/ref hover disabled, math hover unchanged. " +
            "Build with `cargo build --release` in `latex-index/` to enable.",
        );
      }
    } catch {
      sidecar = null;
    }
  }

  return {
    capabilities: {
      textDocumentSync: TextDocumentSyncKind.Full,
      hoverProvider: true,
    },
  };
});

// ══ document sync ══════════════════════════════════════════════════════

docs.onDidOpen((change) => {
  const uri = change.document.uri;
  const text = change.document.getText();
  // `updateFileMacros` is the single forwarding point: it writes the
  // in-process fileCache and forwards to the sidecar (with its own
  // catch on transient errors).  Calling `sidecar.update_file` here
  // as well would race the two writes and trigger the .catch twice.
  updateFileMacros(uri, text);
});

docs.onDidChangeContent((change) => {
  const uri = change.document.uri;
  const text = change.document.getText();
  updateFileMacros(uri, text);
});

docs.onDidClose((change) => {
  // Drop the cached tokeniser spans so closed buffers don't hold memory.
  // The macro cache in preamble.ts is retained (re-opening is cheap there).
  invalidateScannerCache();
  if (sidecar) {
    sidecar.close_file(change.document.uri).catch(() => {
      // ignore
    });
  }
});

docs.listen(connection);

// ══ hover ══════════════════════════════════════════════════════════════

connection.onHover((params) => {
  const doc = docs.get(params.textDocument.uri);
  const text = doc?.getText();
  if (!text) return null;
  return hoverFor(text, params.position, cfg, params.textDocument.uri, sidecar);
});

// ══ shutdown ════════════════════════════════════════════════════════════

async function onShutdown() {
  setSidecar(null);
  if (sidecar) {
    try {
      await sidecar.shutdown();
    } catch {
      // ignore
    }
    sidecar = null;
  }
}

connection.onShutdown(onShutdown);
process.on("SIGINT", () => void onShutdown().then(() => process.exit(0)));
process.on("SIGTERM", () => void onShutdown().then(() => process.exit(0)));

connection.listen();
