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

import { createConnection, ProposedFeatures, TextDocuments, TextDocumentSyncKind } from "vscode-languageserver/node.js";
import { TextDocument } from "vscode-languageserver-textdocument";
import { configFromInit } from "./config.js";
import { hoverFor } from "./hover.js";
import { initWorkspaceMacros, updateFileMacros } from "./preamble.js";
import { invalidateScannerCache } from "./scanner.js";

const connection = createConnection(ProposedFeatures.all);
const docs = new TextDocuments(TextDocument);

let cfg = configFromInit(undefined);

connection.onInitialize((params) => {
  cfg = configFromInit(params.initializationOptions);
  // Auto‑discover \def, \newcommand etc. from every .tex file in the
  // workspace.  Subsequent didOpen/didChange calls keep per‑file caches
  // up to date.
  initWorkspaceMacros(params.rootUri ?? null);
  return {
    capabilities: {
      textDocumentSync: TextDocumentSyncKind.Full,
      hoverProvider: true,
    },
  };
});

// ══ document sync ══════════════════════════════════════════════════════

docs.onDidOpen((change) => {
  updateFileMacros(change.document.uri, change.document.getText());
});

docs.onDidChangeContent((change) => {
  updateFileMacros(change.document.uri, change.document.getText());
});

docs.onDidClose(() => {
  // Drop the cached tokeniser spans so closed buffers don't hold memory.
  // The macro cache in preamble.ts is retained (re-opening is cheap there).
  invalidateScannerCache();
});

docs.listen(connection);

// ══ hover ══════════════════════════════════════════════════════════════

connection.onHover((params) => {
  const doc = docs.get(params.textDocument.uri);
  const text = doc?.getText();
  if (!text) return null;
  return hoverFor(text, params.position, cfg);
});

connection.listen();
