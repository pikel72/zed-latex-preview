//! In‑memory document store.
//!
//! Maps `textDocument.uri` → full buffer text.  Text is received via LSP
//! `textDocument/didOpen` / `didChange` notifications and removed on
//! `didClose`.
//!
//! The store is NOT durably persisted — it is rebuilt from scratch when
//! the LSP is restarted (which happens on extension reload or Zed restart).

export class DocumentStore {
  private docs = new Map<string, string>();

  open(uri: string, text: string) { this.docs.set(uri, text); }
  change(uri: string, text: string) { this.docs.set(uri, text); }
  close(uri: string) { this.docs.delete(uri); }
  get(uri: string): string | undefined { return this.docs.get(uri); }

  /** Return all currently‑open document texts as a flat array. */
  allTexts(): string[] { return [...this.docs.values()]; }
}
