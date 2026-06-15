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
mod index;
mod labels;
mod lsp_codec;
mod macros;
mod workspace;

use crate::cursor::{cursor_context, BufferStore};
use crate::index::{Index, LabelEntry};
use crate::lsp_codec::*;
use crate::workspace::{json_path, normalise_uri};

// ── shared state ───────────────────────────────────────────────────────

struct State {
    started_at: Instant,
    index: Index,
    buffers: BufferStore,
    initialised: bool,
    shutdown: Arc<AtomicBool>,
}

impl State {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            index: Index::new(),
            buffers: BufferStore::new(),
            initialised: false,
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

// ── entry point ────────────────────────────────────────────────────────

fn main() -> anyhow::Result<()> {
    let state = State::new();
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
        Ok(Err(e)) => {
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

fn dispatch(state: &mut State, method: &str, params: &Value) -> anyhow::Result<Value> {
    match method {
        METHOD_INITIALIZE => handle_initialize(state, params),
        METHOD_UPDATE_FILE => handle_update_file(state, params),
        METHOD_CLOSE_FILE => handle_close_file(state, params),
        METHOD_LOOKUP => handle_lookup(state, params),
        METHOD_CURSOR_CONTEXT => handle_cursor_context(state, params),
        METHOD_WORKSPACE_MACROS => handle_workspace_macros(state, params),
        METHOD_PING => handle_ping(state, params),
        _ => Err(anyhow::anyhow!("method not found: {}", method)),
    }
}

// ── handlers ───────────────────────────────────────────────────────────

fn handle_initialize(state: &mut State, params: &Value) -> anyhow::Result<Value> {
    let _root = params.get("rootUri").and_then(|v| v.as_str());
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
