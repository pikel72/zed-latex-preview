//! `latex-index` — Rust sidecar binary for the Zed LaTeX extension.
//!
//! Stdio NDJSON JSON-RPC 2.0 loop.  Reads one JSON message per line from
//! stdin, dispatches to the registered handlers, and writes one JSON
//! response per line to stdout.  See
//! `docs/plan-ref-cite-hover.md` Section 7 for the protocol.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde_json::{json, Value};

mod bibtex;
mod cursor;
mod dict;
mod index;
mod labels;
mod lsp_codec;
mod macros;
mod watcher;
mod workspace;

use crate::cursor::{cursor_context, BufferStore};
use crate::index::{Index, LabelEntry};
use crate::lsp_codec::*;
use crate::workspace::{json_path, normalise_uri};

// ── shared state ───────────────────────────────────────────────────────

struct State {
    started_at: Instant,
    index: Arc<Index>,
    buffers: BufferStore,
    initialised: bool,
    shutdown: Arc<AtomicBool>,
    /// JoinHandle for the spawned file watcher, if any.  Kept so the
    /// thread is joined cleanly on shutdown.
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl State {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            index: Arc::new(Index::new()),
            buffers: BufferStore::new(),
            initialised: false,
            shutdown: Arc::new(AtomicBool::new(false)),
            watcher: None,
        }
    }
}

// ── entry point ────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let mut state = State::new();
    let shutdown = state.shutdown.clone();
    // Shutdown flag is polled in the main loop; signal handling is left to
    // the OS (SIGTERM/SIGINT) and the EOF-on-stdin fallback.
    let _ = shutdown;

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut out = stdout.lock();

    let reader = stdin.lock();
    let mut lines = reader.lines();

    while !state.shutdown.load(Ordering::SeqCst) {
        let line = match lines.next() {
            Some(Ok(l)) => l,
            Some(Err(e)) => {
                let _ = writeln!(stderr, "stdin read error: {e}");
                break;
            }
            None => break,
        };
        if line.is_empty() {
            continue;
        }
        let resp = handle_line(&mut state, &line);
        if let Some(resp) = resp {
            let s = serde_json::to_string(&resp).unwrap_or_else(|e| {
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":e.to_string()}})
                    .to_string()
            });
            if writeln!(out, "{}", s).is_err() {
                break;
            }
            let _ = out.flush();
        }
    }
    Ok(())
}

// ── request handling ───────────────────────────────────────────────────

fn handle_line(state: &mut State, line: &str) -> Option<Value> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let request: Request = match serde_json::from_str(trimmed) {
        Ok(r) => r,
        Err(e) => {
            return Some(serde_json::to_value(ResponseErr::new(
                Value::Null,
                error::PARSE_ERROR,
                format!("parse error: {e}"),
            ))
            .unwrap());
        }
    };

    if request.is_notification() {
        // Notifications (no id) still get handled but produce no response.
        // Borrow instead of clone — handler only reads, request lives for the
        // whole function scope.
        let method = request.method.as_str();
        let id = &request.id;
        let params = &request.params;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = dispatch(state, method, params);
        }));
        // If the notification had an id (rare), error out.
        if !matches!(id, Value::Null) {
            return Some(serde_json::to_value(ResponseErr::new(
                id.clone(),
                error::INVALID_REQUEST,
                "notifications must not include id",
            ))
            .unwrap());
        }
        return None;
    }

    let id = request.id.clone();
    let method = request.method.as_str();
    let params = &request.params;
    let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(state, method, params)
    })) {
        Ok(Ok(v)) => v,
        Ok(Err(RpcError::MethodNotFound(name))) => {
            return Some(serde_json::to_value(ResponseErr::new(
                id,
                error::METHOD_NOT_FOUND,
                format!("method not found: {name}"),
            ))
            .unwrap());
        }
        Ok(Err(RpcError::Internal(e))) => {
            return Some(serde_json::to_value(ResponseErr::new(
                id,
                error::INTERNAL_ERROR,
                e.to_string(),
            ))
            .unwrap());
        }
        Err(_) => {
            return Some(serde_json::to_value(ResponseErr::new(
                id,
                error::INTERNAL_ERROR,
                "internal panic",
            ))
            .unwrap());
        }
    };
    Some(serde_json::to_value(ResponseOk::new(id, result)).unwrap())
}

/// Errors produced by the dispatcher.  We only need to distinguish
/// method-not-found from everything-else; the latter is reported as
/// INTERNAL_ERROR with the anyhow message preserved.
enum RpcError {
    MethodNotFound(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for RpcError {
    fn from(e: anyhow::Error) -> Self {
        RpcError::Internal(e)
    }
}

fn dispatch(state: &mut State, method: &str, params: &Value) -> Result<Value, RpcError> {
    match method {
        METHOD_INITIALIZE => handle_initialize(state, params).map_err(RpcError::from),
        METHOD_UPDATE_FILE => handle_update_file(state, params).map_err(RpcError::from),
        METHOD_CLOSE_FILE => handle_close_file(state, params).map_err(RpcError::from),
        METHOD_LOOKUP => handle_lookup(state, params).map_err(RpcError::from),
        METHOD_CURSOR_CONTEXT => handle_cursor_context(state, params).map_err(RpcError::from),
        METHOD_DOC_LOOKUP => handle_doc_lookup(state, params).map_err(RpcError::from),
        METHOD_WORKSPACE_MACROS => handle_workspace_macros(state, params).map_err(RpcError::from),
        METHOD_PING => handle_ping(state, params).map_err(RpcError::from),
        _ => Err(RpcError::MethodNotFound(method.to_string())),
    }
}

// ── handlers ───────────────────────────────────────────────────────────

fn handle_initialize(state: &mut State, params: &Value) -> anyhow::Result<Value> {
    // Idempotency guard: a defensive second `initialize` is a no-op and
    // does NOT spawn a second watcher.  LSP clients are expected to call
    // `initialize` exactly once, but a leaked thread is expensive and
    // hard to spot.
    if state.initialised {
        return Ok(json!({
            "ok": true,
            "capabilities": { "kinds": ["cite", "ref", "math"] },
            "version": PROTOCOL_VERSION,
        }));
    }
    let root_uri = params.get("rootUri").and_then(|v| v.as_str());
    let version = params
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if version != PROTOCOL_VERSION as u64 {
        return Err(anyhow::anyhow!(
            "protocol version mismatch (got {}, expected {})",
            version,
            PROTOCOL_VERSION
        ));
    }
    // Spawn the file watcher when we have a usable rootUri.  Phase-1
    // tests pass `rootUri: null`; those must still pass without a
    // watcher.
    if let Some(uri) = root_uri {
        if let Some(root_path) = normalise_uri(uri) {
            state.watcher = Some(crate::watcher::spawn_watcher(
                root_path,
                state.index.clone(),
            ));
        }
    }
    // Safe: dispatch takes &mut State, so the write is checked by the borrow checker.
    state.initialised = true;
    Ok(json!({
        "ok": true,
        "capabilities": { "kinds": ["cite", "ref", "math"] },
        "version": PROTOCOL_VERSION,
    }))
}

fn handle_update_file(state: &State, params: &Value) -> anyhow::Result<Value> {
    let uri = params
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("uri required"))?;
    let text = params
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("text required"))?;

    let t0 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let path = normalise_uri(uri)
        .ok_or_else(|| anyhow::anyhow!("bad uri: {}", uri))?;

    // Always remember the buffer text — `cursor_context` needs it.
    state.buffers.put(uri.to_string(), text.to_string());

    // Drop prior entries for this file before re-parsing.
    state.index.remove_file(&path);

    let mut labels_out: Vec<Value> = Vec::new();
    let mut macros_out: Vec<Value> = Vec::new();

    let lower = uri.to_lowercase();
    if lower.ends_with(".bib") {
        bibtex::parse_bibtex(text, &path, &state.index);
    } else if lower.ends_with(".tex") {
        labels::extract_labels(text, &path, &state.index);
        macros::extract_macros(text, &path, &state.index);
        for entry in state.index.labels.iter() {
            if entry.value().file == path {
                labels_out.push(json!({
                    "key": entry.key(),
                    "line": entry.value().line,
                    "env": entry.value().env,
                }));
            }
        }
        for entry in state.index.macros.iter() {
            if entry.value().file == path {
                macros_out.push(json!({
                    "name": entry.key(),
                    "line": 0u32,
                }));
            }
        }
    }

    let t1 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    Ok(json!({
        "ok": true,
        "parse_ms": t1.saturating_sub(t0),
        "labels": labels_out,
        "macros": macros_out,
    }))
}

fn handle_close_file(state: &State, params: &Value) -> anyhow::Result<Value> {
    let uri = params
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("uri required"))?;
    state.buffers.close(uri);
    if let Some(path) = normalise_uri(uri) {
        state.index.remove_file(&path);
    }
    Ok(json!({ "ok": true }))
}

fn handle_lookup(state: &State, params: &Value) -> anyhow::Result<Value> {
    let key = params
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("key required"))?;
    let kind = params
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("kind required"))?;
    match kind {
        "cite" => {
            if let Some(entry) = state.index.bib.get(key) {
                let val = serde_json::to_value(&*entry)?;
                Ok(json!({ "found": true, "entry": val }))
            } else {
                Ok(json!({ "found": false }))
            }
        }
        "ref" => {
            if let Some(entry) = state.index.labels.get(key) {
                let val = label_to_labelref(&entry)?;
                Ok(json!({ "found": true, "entry": val }))
            } else {
                Ok(json!({ "found": false }))
            }
        }
        other => Err(anyhow::anyhow!("unsupported lookup kind: {}", other)),
    }
}

fn label_to_labelref(entry: &LabelEntry) -> anyhow::Result<Value> {
    Ok(json!({
        "key": entry.key,
        "file": json_path(&entry.file),
        "line": entry.line,
        "env": entry.env,
        "math": entry.math,
        "caption": entry.caption,
    }))
}

fn handle_doc_lookup(_state: &State, params: &Value) -> anyhow::Result<Value> {
    // Spec §4.9: bundle a static dictionary, no I/O, no shell-out.  The
    // node side falls back to a math hover when we return `found: false`.
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("name required"))?;
    match dict::lookup(name) {
        Some(entry) => {
            let v: Value = serde_json::to_value(entry)?;
            Ok(json!({ "found": true, "entry": v }))
        }
        None => Ok(json!({ "found": false })),
    }
}

fn handle_cursor_context(state: &State, params: &Value) -> anyhow::Result<Value> {
    let uri = params
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("uri required"))?;
    let offset = params
        .get("offset")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("offset required"))? as usize;
    let ctx = cursor_context(uri, offset, &state.buffers);
    Ok(serde_json::to_value(ctx).context("serialise cursor context")?)
}

fn handle_workspace_macros(state: &State, _params: &Value) -> anyhow::Result<Value> {
    let macros: Vec<Value> = state
        .index
        .macros
        .iter()
        .map(|kv| {
            json!({
                "name": kv.key(),
                "body": kv.value().body,
                "arity": kv.value().arity,
                "file": json_path(&kv.value().file),
            })
        })
        .collect();
    Ok(json!({ "macros": macros }))
}

fn handle_ping(state: &State, _params: &Value) -> anyhow::Result<Value> {
    let uptime_ms = state.started_at.elapsed().as_millis();
    Ok(json!({ "ok": true, "uptime_ms": uptime_ms }))
}
