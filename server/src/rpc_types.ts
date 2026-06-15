//! TypeScript types mirroring the Rust `latex-index` IPC protocol.
//!
//! These types are the contract between `server/src/rust_sidecar.ts` and the
//! `latex-index` binary.  Any drift between this file and the Rust
//! `#[derive(Serialize, Deserialize)]` structs will surface at runtime as
//! missing fields in JSON.  Keep the names and shapes in lock-step with
//! `latex-index/src/{index, cursor, main}.rs`.

// ── data types returned by the sidecar ─────────────────────────────────

/** A BibTeX entry, serialised by `BibEntry` in `latex-index/src/index.rs`. */
export interface BibEntry {
  key: string;
  file: string;
  offset: number;
  fields: Record<string, string>;
  entry_type: string;
}

/** A `\label{...}` site, serialised by `LabelEntry`. */
export interface LabelRef {
  key: string;
  file: string;
  offset: number;
  line: number;
  env: string;
  math?: [number, number] | null;
  caption: string;
  /** Pre-formatted source snippet for hover rendering. */
  snippet: string;
}

/** A `\newcommand`-style macro, serialised by `MacroDef`. */
export interface MacroDef {
  name: string;
  file: string;
  body: string;
  arity: number;
}

// ── request / response shapes ──────────────────────────────────────────

/** `cursor_context` result. */
export interface CursorContext {
  kind: "cite" | "ref" | "math" | "doc" | "none";
  key?: string;
  /** Byte range of the key inside its braces (both exclusive of braces). */
  range?: [number, number];
}

/** `lookup` result for `kind: "cite"`. */
export type CiteLookupResult =
  | { found: true; entry: BibEntry }
  | { found: false };

/** `lookup` result for `kind: "ref"`. */
export type RefLookupResult =
  | { found: true; entry: LabelRef }
  | { found: false };

/** `workspace_macros` result. */
export interface WorkspaceMacrosResult {
  macros: MacroDef[];
}

/** `initialize` result. */
export interface InitializeResult {
  ok: boolean;
  capabilities: { kinds: Array<"cite" | "ref" | "math"> };
  version: number;
}

/** `update_file` result. */
export interface UpdateFileResult {
  ok: boolean;
  parse_ms: number;
  labels: Array<{ key: string; line: number; env: string }>;
  macros: Array<{ name: string; line: number }>;
}

/** `ping` result. */
export interface PingResult {
  ok: boolean;
  uptime_ms: number;
}

// ── `doc_lookup` (Phase 2 §4.9) ────────────────────────────────────────

/** Kind tag for `DocEntry`, serialised by `dict::DictEntry` in Rust. */
export type DocKind = "package" | "command";

/** A single dictionary entry, serialised by `dict::DictEntry`. */
export interface DocEntry {
  kind: DocKind;
  title: string;
  short: string;
  docs?: string;
}

/** `doc_lookup` params. */
export interface DocLookupRequest {
  name: string;
}

/** `doc_lookup` result. */
export type DocLookupResult =
  | { found: true; entry: DocEntry }
  | { found: false };

// ── JSON-RPC 2.0 envelope ──────────────────────────────────────────────

export interface RpcRequest<P = unknown> {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params: P;
}

export interface RpcNotification<P = unknown> {
  jsonrpc: "2.0";
  method: string;
  params: P;
}

export interface RpcResponseOk<R = unknown> {
  jsonrpc: "2.0";
  id: number;
  result: R;
}

export interface RpcResponseErr {
  jsonrpc: "2.0";
  id: number;
  error: { code: number; message: string; data?: unknown };
}

export type RpcMessage =
  | RpcRequest
  | RpcNotification
  | RpcResponseOk
  | RpcResponseErr;
