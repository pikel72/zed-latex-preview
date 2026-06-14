//! LSP server entry point.
//!
//! Wires up the `vscode‑languageserver` transport to our internal modules:
//!
//! ```text
//!                    ┌────────────────┐
//! textDocument/      │   server.ts    │
//! didOpen ──────────→│                │
//! didChange ────────→│  DocumentStore │──── hoverFor() ──── HoverResult
//! didClose ─────────→│                │
//!                    │  preamble.ts   │──── initWorkspaceMacros()
//!                    │                │──── updateFileMacros()
//!                    └────────────────┘
//! ```
//!
//! The server responds to `textDocument/hover` requests only.  It does not
//! register completions, diagnostics, or any other LSP capability.

import { createConnection, ProposedFeatures, TextDocuments, TextDocumentSyncKind } from "vscode-languageserver/node.js";
import { TextDocument } from "vscode-languageserver-textdocument";
import { configFromInit } from "./config.js";
import { DocumentStore } from "./documents.js";
import { hoverFor } from "./hover.js";
import { initWorkspaceMacros, updateFileMacros } from "./preamble.js";

const connection = createConnection(ProposedFeatures.all);
const docs = new TextDocuments(TextDocument);
const store = new DocumentStore();

let cfg = configFromInit(process.env.LATEX_PREVIEW_INIT);

connection.onInitialize((params) => {
  cfg = configFromInit(params.initializationOptions ?? process.env.LATEX_PREVIEW_INIT);
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
  store.open(change.document.uri, change.document.getText());
  updateFileMacros(change.document.uri, change.document.getText());
});

docs.onDidChangeContent((change) => {
  store.change(change.document.uri, change.document.getText());
  updateFileMacros(change.document.uri, change.document.getText());
});

docs.onDidClose((change) => {
  store.close(change.document.uri);
  // Macros from closed files are kept — the user may reopen later.
});

docs.listen(connection);

// ══ hover ══════════════════════════════════════════════════════════════

connection.onHover((params) => {
  const text = store.get(params.textDocument.uri);
  if (!text) return null;
  return hoverFor(text, params.position, cfg);
});

connection.listen();
