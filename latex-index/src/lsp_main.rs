//! Rust-native LSP entry point for LaTeX Preview.
//!
//! This is the Rust-primary runtime: Zed talks LSP stdio directly to this
//! binary.  The older `main.rs` sidecar protocol remains during the migration
//! so existing tests and fallback paths can continue to run.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use base64::Engine;
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    MarkupContent, MarkupKind, Position, Range, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind,
};
use serde_json::Value;

mod bibtex;
mod cursor;
mod dict;
mod index;
mod labels;
mod macros;
mod watcher;
mod workspace;

use crate::cursor::{cursor_context, BufferStore};
use crate::index::{BibEntry, Index, LabelEntry};
use crate::workspace::{collect_files, json_path, normalise_uri};

fn main() -> anyhow::Result<()> {
    let (connection, io_threads) = Connection::stdio();
    run_server(connection)?;
    // `run_server` consumes the connection by value, so its
    // `Sender`/`Receiver` clones are gone by the time we get here.  That
    // closes the writer channel and lets the writer thread exit; otherwise
    // `io_threads.join()` deadlocks waiting on a still-open sender.
    io_threads.join()?;
    Ok(())
}

fn run_server(connection: Connection) -> anyhow::Result<()> {
    let mut server = LspServer::new();

    let (initialize_id, initialize_params) = connection.initialize_start()?;
    let initialize_params: InitializeParams =
        serde_json::from_value(initialize_params).context("parse initialize params")?;
    server.initialize(&initialize_params);
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        ..Default::default()
    };
    let result = InitializeResult {
        capabilities,
        server_info: Some(ServerInfo {
            name: "latex-preview-lsp".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };
    connection.initialize_finish(initialize_id, serde_json::to_value(result)?)?;

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
                server.handle_request(&connection, req)?;
            }
            Message::Notification(note) => server.handle_notification(note)?,
            Message::Response(_) => {}
        }
    }

    // Drop the LSP server (and with it the watcher handle + render worker
    // sender) before returning, so its destructors run while the connection
    // is still around to log any final errors.  `connection` itself drops
    // when this function returns.
    drop(server);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ColorMode {
    Auto,
    Black,
    White,
}

#[derive(Debug, Clone)]
struct PreviewConfig {
    enabled: bool,
    max_formula_length: usize,
    timeout_ms: u64,
    scale: f64,
    color: ColorMode,
    enabled_cite_preview: bool,
    enabled_ref_preview: bool,
    enabled_doc_preview: bool,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_formula_length: 2000,
            timeout_ms: 1500,
            scale: 1.4,
            color: ColorMode::Auto,
            enabled_cite_preview: true,
            enabled_ref_preview: true,
            enabled_doc_preview: true,
        }
    }
}

impl PreviewConfig {
    fn from_value(value: Option<&Value>) -> Self {
        let mut cfg = Self::default();
        let Some(obj) = value.and_then(|v| v.as_object()) else {
            return cfg;
        };
        if let Some(v) = obj.get("enabled").and_then(|v| v.as_bool()) {
            cfg.enabled = v;
        }
        if let Some(v) = obj.get("maxFormulaLength").and_then(|v| v.as_u64()) {
            cfg.max_formula_length = v as usize;
        }
        if let Some(v) = obj.get("timeoutMs").and_then(|v| v.as_u64()) {
            cfg.timeout_ms = v;
        }
        if let Some(v) = obj.get("scale").and_then(|v| v.as_f64()) {
            cfg.scale = v;
        }
        if let Some(v) = obj.get("color").and_then(|v| v.as_str()) {
            cfg.color = match v {
                "black" => ColorMode::Black,
                "white" => ColorMode::White,
                _ => ColorMode::Auto,
            };
        }
        if let Some(v) = obj.get("enabledCitePreview").and_then(|v| v.as_bool()) {
            cfg.enabled_cite_preview = v;
        }
        if let Some(v) = obj.get("enabledRefPreview").and_then(|v| v.as_bool()) {
            cfg.enabled_ref_preview = v;
        }
        if let Some(v) = obj.get("enabledDocPreview").and_then(|v| v.as_bool()) {
            cfg.enabled_doc_preview = v;
        }
        cfg
    }
}

struct LspServer {
    index: Arc<Index>,
    buffers: BufferStore,
    cfg: PreviewConfig,
    renderer: MathJaxRenderer,
    /// Workspace file watcher.  Stored so its `Drop` runs on `LspServer`
    /// teardown, signalling the watcher thread to exit and joining it
    /// before the process exits.  See `watcher::WatcherHandle`.
    watcher: Option<watcher::WatcherHandle>,
}

impl LspServer {
    fn new() -> Self {
        Self {
            index: Arc::new(Index::new()),
            buffers: BufferStore::new(),
            cfg: PreviewConfig::default(),
            renderer: MathJaxRenderer::new(),
            watcher: None,
        }
    }

    fn initialize(&mut self, params: &InitializeParams) {
        self.cfg = PreviewConfig::from_value(params.initialization_options.as_ref());
        if let Some(root_uri) = initialize_root_uri(params) {
            if let Some(root) = normalise_uri(&root_uri) {
                self.prime_workspace(&root);
                self.watcher = Some(watcher::spawn_watcher(root, self.index.clone()));
            }
        }
    }

    fn handle_request(&mut self, connection: &Connection, req: Request) -> anyhow::Result<()> {
        match req.method.as_str() {
            "textDocument/hover" => {
                let id = req.id.clone();
                let params: HoverParams =
                    serde_json::from_value(req.params).context("parse hover params")?;
                let result = self.hover(params)?;
                let response = Response {
                    id,
                    result: Some(serde_json::to_value(result)?),
                    error: None,
                };
                connection.sender.send(Message::Response(response))?;
            }
            _ => {
                let response = Response {
                    id: req.id,
                    result: None,
                    error: Some(lsp_server::ResponseError {
                        code: lsp_server::ErrorCode::MethodNotFound as i32,
                        message: format!("method not found: {}", req.method),
                        data: None,
                    }),
                };
                connection.sender.send(Message::Response(response))?;
            }
        }
        Ok(())
    }

    fn handle_notification(&mut self, note: Notification) -> anyhow::Result<()> {
        match note.method.as_str() {
            "textDocument/didOpen" => {
                let params: DidOpenTextDocumentParams =
                    serde_json::from_value(note.params).context("parse didOpen params")?;
                self.update_file(
                    params.text_document.uri.as_str(),
                    &params.text_document.text,
                );
            }
            "textDocument/didChange" => {
                let params: DidChangeTextDocumentParams =
                    serde_json::from_value(note.params).context("parse didChange params")?;
                if let Some(change) = params.content_changes.into_iter().last() {
                    self.update_file(params.text_document.uri.as_str(), &change.text);
                }
            }
            "textDocument/didClose" => {
                let params: DidCloseTextDocumentParams =
                    serde_json::from_value(note.params).context("parse didClose params")?;
                self.close_file(params.text_document.uri.as_str());
            }
            _ => {}
        }
        Ok(())
    }

    fn prime_workspace(&self, root: &Path) {
        for path in collect_files(root) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            self.index_path(&path, &text);
        }
    }

    fn update_file(&self, uri: &str, text: &str) {
        self.buffers.put(uri.to_string(), text.to_string());
        let Some(path) = normalise_uri(uri) else {
            return;
        };
        self.index.remove_file(&path);
        self.index_path(&path, text);
    }

    fn close_file(&self, uri: &str) {
        // Drop the in-memory buffer only.  The workspace index reflects
        // what is on disk (and is kept fresh by the file watcher); closing a
        // buffer is not a disk change, so other files that `\ref`/`\cite`
        // into this one must keep resolving.  If the file is truly gone the
        // next watcher event — or a re-open — will reconcile.
        self.buffers.close(uri);
    }

    fn index_path(&self, path: &Path, text: &str) {
        let lower = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if lower.ends_with(".bib") {
            bibtex::parse_bibtex(text, path, &self.index);
        } else if lower.ends_with(".tex") {
            labels::extract_labels(text, path, &self.index);
            macros::extract_macros(text, path, &self.index);
        }
    }

    fn hover(&self, params: HoverParams) -> anyhow::Result<Option<Hover>> {
        if !self.cfg.enabled {
            return Ok(None);
        }
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let uri_str = uri.as_str();
        let Some(text) = self.buffers.get(uri_str) else {
            return Ok(None);
        };
        let offset = position_to_offset(&text, position);
        let ctx = cursor_context(uri_str, offset, &self.buffers);

        if ctx.kind == "cite" && self.cfg.enabled_cite_preview {
            if let Some(key) = ctx.key.as_deref() {
                let hover = self.cite_hover(key, &text, ctx.range)?;
                if hover.is_some() {
                    return Ok(hover);
                }
            }
        }
        if ctx.kind == "ref" && self.cfg.enabled_ref_preview {
            if let Some(key) = ctx.key.as_deref() {
                let hover = self.ref_hover(key, &text, ctx.range)?;
                if hover.is_some() {
                    return Ok(hover);
                }
            }
        }
        if ctx.kind == "doc" && self.cfg.enabled_doc_preview {
            if let Some(key) = ctx.key.as_deref() {
                let hover = self.doc_hover(key)?;
                if hover.is_some() {
                    return Ok(hover);
                }
            }
        }

        self.math_hover(&text, offset)
    }

    fn cite_hover(
        &self,
        key: &str,
        text: &str,
        range: Option<[usize; 2]>,
    ) -> anyhow::Result<Option<Hover>> {
        let value = if let Some(entry) = self.index.bib.get(key) {
            format_cite_hover(&entry)
        } else {
            "_(citation not found)_".to_string()
        };
        Ok(Some(markdown_hover(
            value,
            range.map(|r| byte_range_to_lsp_range(text, r)),
        )))
    }

    fn ref_hover(
        &self,
        key: &str,
        text: &str,
        range: Option<[usize; 2]>,
    ) -> anyhow::Result<Option<Hover>> {
        let Some(entry) = self.index.labels.get(key) else {
            return Ok(None);
        };
        let value = format_ref_hover(&entry, &self.renderer, &self.cfg);
        Ok(Some(markdown_hover(
            value,
            range.map(|r| byte_range_to_lsp_range(text, r)),
        )))
    }

    fn doc_hover(&self, key: &str) -> anyhow::Result<Option<Hover>> {
        let Some(entry) = dict::lookup(key) else {
            return Ok(None);
        };
        let kind = match entry.kind {
            dict::DocKind::Package => "package",
            dict::DocKind::Command => "command",
        };
        let body = entry.docs.unwrap_or(entry.short);
        Ok(Some(markdown_hover(
            format!("**{}** ({})\n\n{}", entry.title, kind, body),
            None,
        )))
    }

    fn math_hover(&self, text: &str, offset: usize) -> anyhow::Result<Option<Hover>> {
        let Some(region) = find_math_region(text, offset, self.cfg.max_formula_length) else {
            return Ok(None);
        };
        let mut macros = workspace_macros(&self.index);
        for (name, def) in document_macros(text) {
            macros.insert(name, def);
        }
        let expanded = expand_macros(&region.source, &macros);
        let value = match self.renderer.render(RenderRequest {
            source: expanded.clone(),
            display: region.display,
            scale: self.cfg.scale,
            color: self.cfg.color,
            timeout_ms: self.cfg.timeout_ms,
        }) {
            RenderResult::Ok { svg } => {
                let data = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
                format!("![formula](data:image/svg+xml;base64,{data})")
            }
            RenderResult::Err { error } => {
                let _ = error;
                // Render failed (timeout/parse error) but expansion succeeded;
                // show the expanded source so the user sees resolved macros
                // (e.g. \Omega) rather than the raw \O shorthand.
                format!("```latex\n{}\n```", expanded)
            }
        };
        Ok(Some(markdown_hover(
            value,
            Some(Range {
                start: offset_to_position(text, region.start),
                end: offset_to_position(text, region.end),
            }),
        )))
    }
}

fn initialize_root_uri(params: &InitializeParams) -> Option<String> {
    if let Some(folder) = params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first())
    {
        return Some(folder.uri.to_string());
    }
    #[allow(deprecated)]
    params.root_uri.as_ref().map(|uri| uri.to_string())
}

fn markdown_hover(value: String, range: Option<Range>) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range,
    }
}

fn byte_range_to_lsp_range(text: &str, range: [usize; 2]) -> Range {
    Range {
        start: offset_to_position(text, range[0]),
        end: offset_to_position(text, range[1]),
    }
}

/// LSP positions use UTF-16 code units per spec; convert a `Position` to a
/// byte offset by walking the text in UTF-16 increments.
fn position_to_offset(text: &str, pos: Position) -> usize {
    let mut line = 0u32;
    let mut ch = 0u32;
    for (i, c) in text.char_indices() {
        if line == pos.line && ch == pos.character {
            return i;
        }
        if c == '\n' {
            line += 1;
            ch = 0;
        } else {
            ch += c.len_utf16() as u32;
        }
    }
    // Cursor at end of document.
    if line == pos.line && ch == pos.character {
        return text.len();
    }
    text.len()
}

/// Convert a byte offset into an LSP `Position` measured in UTF-16 code units.
fn offset_to_position(text: &str, offset: usize) -> Position {
    let mut line = 0u32;
    let mut ch = 0u32;
    for (i, c) in text.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            ch = 0;
        } else {
            ch += c.len_utf16() as u32;
        }
    }
    Position { line, character: ch }
}

// ── Math region scanner ───────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MathRegion {
    start: usize,
    end: usize,
    source: String,
    display: bool,
}

fn find_math_region(text: &str, offset: usize, max_len: usize) -> Option<MathRegion> {
    let bytes = text.as_bytes();
    let off = offset.min(bytes.len());
    for span in tokenize_math(bytes) {
        if off >= span.open && off < span.end {
            if span.body_end.saturating_sub(span.body_start) > max_len {
                return None;
            }
            return Some(MathRegion {
                start: span.open,
                end: span.end,
                source: text[span.body_start..span.body_end].to_string(),
                display: span.display,
            });
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct MathSpan {
    open: usize,
    end: usize,
    body_start: usize,
    body_end: usize,
    display: bool,
}

const MATH_ENVS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
];
const VERBATIM_ENVS: &[&str] = &["verbatim", "lstlisting", "minted"];

fn tokenize_math(text: &[u8]) -> Vec<MathSpan> {
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let b = text[i];
        if b == b'%' && !is_escaped(text, i) {
            i = skip_to_eol(text, i + 1);
            continue;
        }
        if b == b'\\' && starts_with(text, i, b"\\begin") && !is_escaped(text, i) {
            if let Some((name, tag_end)) = read_env_tag(text, i, "begin") {
                let close_tag = format!("\\end{{{}}}", name);
                let close = find_subslice(text, tag_end, close_tag.as_bytes());
                let body_end = close.unwrap_or(text.len());
                let end = close.map(|p| p + close_tag.len()).unwrap_or(text.len());
                if VERBATIM_ENVS.contains(&name) {
                    i = end;
                    continue;
                }
                if MATH_ENVS.contains(&name) {
                    spans.push(MathSpan {
                        open: i,
                        end,
                        body_start: tag_end,
                        body_end,
                        display: true,
                    });
                    i = end;
                    continue;
                }
            }
        }
        if b == b'$' && !is_escaped(text, i) {
            let is_double = text.get(i + 1) == Some(&b'$');
            let width = if is_double { 2 } else { 1 };
            let close = find_dollar_closer(text, i + width, is_double);
            let body_end = close.unwrap_or(text.len());
            let end = close.map(|p| p + width).unwrap_or(text.len());
            spans.push(MathSpan {
                open: i,
                end,
                body_start: i + width,
                body_end,
                display: is_double,
            });
            i = end;
            continue;
        }
        if b == b'\\' && !is_escaped(text, i) {
            let next = text.get(i + 1).copied();
            if next == Some(b'(') || next == Some(b'[') {
                let display = next == Some(b'[');
                let close_delim: &[u8] = if display { b"\\]" } else { b"\\)" };
                let close = find_subslice(text, i + 2, close_delim);
                let body_end = close.unwrap_or(text.len());
                let end = close.map(|p| p + close_delim.len()).unwrap_or(text.len());
                spans.push(MathSpan {
                    open: i,
                    end,
                    body_start: i + 2,
                    body_end,
                    display,
                });
                i = end;
                continue;
            }
        }
        i += 1;
    }
    spans
}

fn starts_with(text: &[u8], at: usize, needle: &[u8]) -> bool {
    at + needle.len() <= text.len() && &text[at..at + needle.len()] == needle
}

fn is_escaped(text: &[u8], i: usize) -> bool {
    let mut bs = 0usize;
    let mut k = i;
    while k > 0 {
        k -= 1;
        if text[k] == b'\\' {
            bs += 1;
        } else {
            break;
        }
    }
    bs % 2 == 1
}

fn skip_to_eol(text: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < text.len() && text[i] != b'\n' {
        i += 1;
    }
    if i < text.len() {
        i + 1
    } else {
        text.len()
    }
}

fn read_env_tag<'a>(text: &'a [u8], at: usize, kind: &str) -> Option<(&'a str, usize)> {
    let want: &[u8] = if kind == "begin" {
        b"\\begin"
    } else {
        b"\\end"
    };
    if !starts_with(text, at, want) {
        return None;
    }
    let mut i = at + want.len();
    while i < text.len() && (text[i] == b' ' || text[i] == b'\t') {
        i += 1;
    }
    if text.get(i) != Some(&b'{') {
        return None;
    }
    i += 1;
    let start = i;
    while i < text.len() && text[i] != b'}' {
        i += 1;
    }
    if i >= text.len() {
        return None;
    }
    Some((std::str::from_utf8(&text[start..i]).ok()?, i + 1))
}

fn find_dollar_closer(text: &[u8], from: usize, want_double: bool) -> Option<usize> {
    let mut i = from;
    while i < text.len() {
        let b = text[i];
        if b == b'\\' {
            i = (i + 2).min(text.len());
            continue;
        }
        if b == b'%' {
            i = skip_to_eol(text, i);
            continue;
        }
        if b == b'$' {
            let is_double = text.get(i + 1) == Some(&b'$');
            if (want_double && is_double) || (!want_double && !is_double) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn find_subslice(text: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut i = from;
    while i + needle.len() <= text.len() {
        if &text[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

// ── Macro expansion ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct MacroValue {
    body: String,
    arity: u32,
}

fn workspace_macros(index: &Index) -> BTreeMap<String, MacroValue> {
    let mut out = BTreeMap::new();
    for item in index.macros.iter() {
        out.insert(
            item.key().clone(),
            MacroValue {
                body: item.value().body.clone(),
                arity: item.value().arity,
            },
        );
    }
    out
}

fn document_macros(text: &str) -> BTreeMap<String, MacroValue> {
    let tmp = Index::new();
    macros::extract_macros(text, Path::new("<buffer>"), &tmp);
    workspace_macros(&tmp)
}

fn expand_macros(source: &str, macros: &BTreeMap<String, MacroValue>) -> String {
    let mut out = source.to_string();
    for (name, def) in macros {
        let head = format!("\\{name}");
        let mut result = String::new();
        let mut cursor = 0usize;
        while cursor < out.len() {
            let Some(rel) = out[cursor..].find(&head) else {
                result.push_str(&out[cursor..]);
                break;
            };
            let idx = cursor + rel;
            let next = out.as_bytes().get(idx + head.len()).copied();
            if matches!(next, Some(b) if b.is_ascii_alphabetic() || b == b'@') {
                result.push_str(&out[cursor..idx + head.len()]);
                cursor = idx + head.len();
                continue;
            }
            result.push_str(&out[cursor..idx]);
            if def.arity == 0 {
                result.push_str(&def.body);
                cursor = idx + head.len();
                continue;
            }
            let mut pos = idx + head.len();
            let mut args = Vec::new();
            let mut ok = true;
            for _ in 0..def.arity {
                if let Some((arg, end)) = read_macro_arg(&out, pos) {
                    args.push(arg);
                    pos = end;
                } else {
                    ok = false;
                    break;
                }
            }
            if !ok {
                result.push_str(&out[idx..pos]);
                cursor = pos;
                continue;
            }
            let mut expansion = def.body.clone();
            for (i, arg) in args.iter().enumerate() {
                expansion = expansion.replace(&format!("#{}", i + 1), arg);
            }
            result.push_str(&expansion);
            cursor = pos;
        }
        out = result;
    }
    out
}

fn read_macro_arg(src: &str, at: usize) -> Option<(String, usize)> {
    let bytes = src.as_bytes();
    let mut i = at;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'{' {
        let (a, b, end) = read_balanced(bytes, i)?;
        return Some((src[a..b].to_string(), end));
    }
    let start = i;
    while i < bytes.len() && bytes[i] != b' ' && bytes[i] != b'\t' && bytes[i] != b'{' {
        i += 1;
    }
    Some((src[start..i].to_string(), i))
}

fn read_balanced(text: &[u8], at: usize) -> Option<(usize, usize, usize)> {
    if text.get(at) != Some(&b'{') {
        return None;
    }
    let mut depth = 1i32;
    let mut i = at + 1;
    while i < text.len() {
        match text[i] {
            b'\\' => i = (i + 2).min(text.len()),
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some((at + 1, i - 1, i));
                }
            }
            _ => i += 1,
        }
    }
    None
}

// ── Hover formatting ──────────────────────────────────────────────────

fn format_cite_hover(entry: &BibEntry) -> String {
    let mut lines = Vec::new();
    if let Some(heading) = cite_heading(entry) {
        lines.push(heading);
    }
    for (label, key) in [
        ("Authors", "author"),
        ("Editor", "editor"),
        ("Publisher", "publisher"),
        ("Journal", "journal"),
        ("Booktitle", "booktitle"),
        ("Volume", "volume"),
        ("Number", "number"),
        ("Pages", "pages"),
        ("Series", "series"),
        ("Edition", "edition"),
        ("Address", "address"),
        ("DOI", "doi"),
        ("URL", "url"),
        ("Year", "year"),
    ] {
        let Some(raw) = entry.fields.get(key) else {
            continue;
        };
        let cleaned = clean_field_value(raw);
        if cleaned.is_empty() {
            continue;
        }
        if key == "year" && lines.first().is_some_and(|h| h.contains(&cleaned)) {
            continue;
        }
        if key == "author" && lines.first().is_some_and(|h| h.starts_with(&cleaned)) {
            continue;
        }
        lines.push(format!("{label}: {cleaned}"));
    }
    if let Some(abs) = entry.fields.get("abstract").map(|v| clean_field_value(v)) {
        if !abs.is_empty() {
            lines.push(String::new());
            lines.push(format!("> {abs}"));
        }
    }
    lines.push(String::new());
    lines.push(format!("File: {}:1", short_path(&json_path(&entry.file))));
    lines.join("\n")
}

fn cite_heading(entry: &BibEntry) -> Option<String> {
    let author = entry.fields.get("author").map(|v| clean_field_value(v));
    let year = entry.fields.get("year").map(|v| v.trim().to_string());
    let title = entry.fields.get("title").map(|v| clean_field_value(v));
    match (
        author.filter(|s| !s.is_empty()),
        year.filter(|s| !s.is_empty()),
        title.filter(|s| !s.is_empty()),
    ) {
        (Some(a), Some(y), Some(t)) => Some(format!("{a} {y} - *{t}*")),
        (Some(a), _, Some(t)) => Some(format!("{a} - *{t}*")),
        (_, Some(y), Some(t)) => Some(format!("{y} - *{t}*")),
        (_, _, Some(t)) => Some(format!("*{t}*")),
        (Some(a), _, _) => Some(a),
        (_, Some(y), _) => Some(y),
        _ => None,
    }
}

fn clean_field_value(raw: &str) -> String {
    let mut s = strip_outer_braces(raw.trim()).to_string();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s
}

fn strip_outer_braces(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'{' || *bytes.last().unwrap_or(&0) != b'}' {
        return s;
    }
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 && i != bytes.len() - 1 {
                    return s;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    if depth == 0 {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn format_ref_hover(entry: &LabelEntry, renderer: &MathJaxRenderer, cfg: &PreviewConfig) -> String {
    let header = ref_header(entry);
    let location = format!("{}:{}", short_path(&json_path(&entry.file)), entry.line + 1);
    let snippet = if labels::MATH_ENVS.contains(&entry.env.as_str()) {
        let source = wrap_ref_math(&entry.env, &entry.snippet);
        match renderer.render(RenderRequest {
            source,
            display: true,
            scale: cfg.scale,
            color: cfg.color,
            timeout_ms: cfg.timeout_ms,
        }) {
            RenderResult::Ok { svg } => {
                let data = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
                format!("![formula](data:image/svg+xml;base64,{data})")
            }
            RenderResult::Err { error } => {
                let _ = error;
                fenced_block(&entry.snippet)
            }
        }
    } else {
        fenced_block(&entry.snippet)
    };
    [header, snippet, location].join("\n\n")
}

fn ref_header(entry: &LabelEntry) -> String {
    if entry.env == "section" {
        if entry.caption.is_empty() {
            format!("Section: {}", entry.key)
        } else {
            format!("Section: {}", entry.caption)
        }
    } else if labels::MATH_ENVS.contains(&entry.env.as_str()) {
        title_case(&entry.env)
    } else if !entry.env.is_empty() {
        let title = title_case(&entry.env);
        if entry.caption.is_empty() {
            title
        } else {
            format!("{title}: {}", entry.caption)
        }
    } else {
        entry.key.clone()
    }
}

fn title_case(s: &str) -> String {
    let base = s.strip_suffix('*').unwrap_or(s);
    let mut chars = base.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn wrap_ref_math(env: &str, body: &str) -> String {
    match env {
        "equation" | "equation*" => body.to_string(),
        "gather" | "gather*" => format!("\\begin{{gathered}}\n{body}\n\\end{{gathered}}"),
        _ => format!("\\begin{{aligned}}\n{body}\n\\end{{aligned}}"),
    }
}

fn fenced_block(snippet: &str) -> String {
    let fence = fence_for(snippet);
    format!("{fence}\n{snippet}\n{fence}")
}

fn fence_for(snippet: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for b in snippet.bytes() {
        if b == b'`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(3.max(longest + 1))
}

fn short_path(path: &str) -> String {
    let norm = path.replace('\\', "/");
    let parts = norm
        .split('/')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if parts.len() <= 3 {
        norm
    } else {
        format!(".../{}", parts[parts.len() - 2..].join("/"))
    }
}

// ── Renderer ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct RenderRequest {
    source: String,
    display: bool,
    scale: f64,
    color: ColorMode,
    timeout_ms: u64,
}

enum RenderResult {
    Ok { svg: String },
    Err { error: String },
}

struct MathJaxRenderer {
    /// Job queue for the single dedicated render worker.  Rendering always
    /// happens on this one thread, so a render timeout can never leak a fresh
    /// OS thread per hover: a timed-out request simply abandons its result
    /// while the worker finishes (or the next request queues behind it).
    jobs: std::sync::mpsc::Sender<RenderJob>,
    cache: RenderCache,
}

struct RenderJob {
    source: String,
    options: mathjax_svg_rs::Options,
    reply: std::sync::mpsc::Sender<Result<String, String>>,
}

/// Bounded LRU-ish render cache.  Keyed by the exact inputs that change the
/// produced SVG; the value is the *post-processed* SVG (color injected, ex→px
/// rewritten, sanitised) so it is reusable across hover sites.
///
/// Two-level `Mutex<Vec<...>>` with FIFO eviction keeps it allocation-free in
/// steady state and lock-held-for microseconds; a real LRU crate is not worth
/// the dependency for an N≈64 hover cache.
struct RenderCache {
    inner: std::sync::Mutex<Vec<(RenderKey, String)>>,
    capacity: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RenderKey {
    source: String,
    display: bool,
    scale_scale_milli: u64,
    color: ColorMode,
}

impl RenderCache {
    fn new(capacity: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(Vec::with_capacity(capacity)),
            capacity,
        }
    }

    fn get(&self, key: &RenderKey) -> Option<String> {
        let mut g = self.inner.lock().expect("render-cache poisoned");
        if let Some(pos) = g.iter().position(|(k, _)| k == key) {
            // Move-to-front: most-recent hover is most likely to recur.
            let entry = g.remove(pos);
            g.push(entry);
            Some(g.last().unwrap().1.clone())
        } else {
            None
        }
    }

    fn put(&self, key: RenderKey, svg: String) {
        let mut g = self.inner.lock().expect("render-cache poisoned");
        if let Some(pos) = g.iter().position(|(k, _)| *k == key) {
            g[pos].1 = svg;
            return;
        }
        if g.len() >= self.capacity {
            g.remove(0);
        }
        g.push((key, svg));
    }
}

const DEFAULT_EX_PX: f64 = 8.0;
const SVG_PAD_PX: f64 = 8.0;
const RENDER_CACHE_CAPACITY: usize = 64;

impl MathJaxRenderer {
    fn new() -> Self {
        let inner = Arc::new(mathjax_svg_rs::MathJax::new());
        // Initialise the embedded MathJax worker during LSP startup instead
        // of charging that cost to the first hover request, where it would
        // otherwise trip the user-facing render timeout.
        let _ = inner.render_tex("x", &mathjax_svg_rs::Options::default());

        let (tx, rx) = std::sync::mpsc::channel::<RenderJob>();
        let worker_inner = inner.clone();
        // One dedicated render thread for the lifetime of the server.  Boa is
        // not Send across a true thread pool without care, and a single worker
        // is enough for hover-rate work once the cache is warm.
        std::thread::Builder::new()
            .name("latex-preview-render".into())
            .spawn(move || {
                for job in rx {
                    let result = worker_inner
                        .render_tex(&job.source, &job.options)
                        .map_err(|e| e.to_string());
                    // Reply errors are fine: the caller already timed out.
                    let _ = job.reply.send(result);
                }
            })
            .expect("spawn render worker");

        Self {
            jobs: tx,
            cache: RenderCache::new(RENDER_CACHE_CAPACITY),
        }
    }

    fn render(&self, req: RenderRequest) -> RenderResult {
        let key = RenderKey {
            source: req.source.clone(),
            display: req.display,
            // f64 has no Eq/Hash; quantise to 0.001 so 1.4 and 1.4001 collapse.
            scale_scale_milli: (req.scale * 1000.0).round() as u64,
            color: req.color,
        };
        if let Some(svg) = self.cache.get(&key) {
            return RenderResult::Ok { svg };
        }

        let font_size = 16.0 * req.scale.max(0.1);
        let options = mathjax_svg_rs::Options {
            font_size,
            ..Default::default()
        };
        let source = if req.display {
            format!("\\displaystyle {}", req.source)
        } else {
            req.source.clone()
        };
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let job = RenderJob {
            source,
            options,
            reply: reply_tx,
        };
        // Worker is alive for the whole server lifetime; send only fails at
        // shutdown, which we treat as a render error.
        if self.jobs.send(job).is_err() {
            return RenderResult::Err {
                error: "render worker unavailable".to_string(),
            };
        }

        match reply_rx.recv_timeout(Duration::from_millis(req.timeout_ms.max(1))) {
            Ok(Ok(svg)) => {
                if svg.contains("data-mjx-error") || svg.contains("mtext\" fill=\"red\"") {
                    RenderResult::Err {
                        error: "mathjax parse error".to_string(),
                    }
                } else {
                    let svg = inject_svg_color(svg, req.color);
                    let svg = normalize_svg_size(svg, req.scale);
                    let svg = sanitize_svg_for_zed(svg);
                    self.cache.put(key, svg.clone());
                    RenderResult::Ok { svg }
                }
            }
            Ok(Err(error)) => {
                eprintln!("latex-preview: render error: {error}");
                RenderResult::Err { error }
            }
            Err(e) => {
                eprintln!("latex-preview: render timeout after {}ms: {e}", req.timeout_ms);
                RenderResult::Err {
                    error: format!("mathjax timeout/error: {e}"),
                }
            }
        }
    }
}

fn inject_svg_color(svg: String, color: ColorMode) -> String {
    let color = match color {
        ColorMode::Auto => "currentColor",
        ColorMode::Black => "black",
        ColorMode::White => "white",
    };
    if let Some(pos) = svg.find("<svg") {
        if let Some(end_rel) = svg[pos..].find('>') {
            let insert = pos + end_rel;
            let mut out = String::with_capacity(svg.len() + color.len() + 10);
            out.push_str(&svg[..insert]);
            out.push_str(&format!(" color=\"{color}\""));
            out.push_str(&svg[insert..]);
            return out;
        }
    }
    svg
}

fn normalize_svg_size(svg: String, scale: f64) -> String {
    let ex_px = DEFAULT_EX_PX * scale.max(0.1);
    rewrite_ex_dimension_attrs(&svg, ex_px)
}

fn rewrite_ex_dimension_attrs(svg: &str, ex_px: f64) -> String {
    let bytes = svg.as_bytes();
    let mut out = String::with_capacity(svg.len());
    let mut i = 0usize;
    let mut last_emit = 0usize;
    while i < bytes.len() {
        let Some(name_len) = ex_dimension_attr_name_len(bytes, i) else {
            i += 1;
            continue;
        };
        let quote_at = i + name_len + 1;
        let Some(quote @ (b'"' | b'\'')) = bytes.get(quote_at).copied() else {
            i += 1;
            continue;
        };
        let value_start = quote_at + 1;
        let Some((value_end, value)) = read_ex_number_attr_value(svg, value_start, quote) else {
            i += 1;
            continue;
        };

        let px = (value * ex_px).round() + SVG_PAD_PX;
        out.push_str(&svg[last_emit..value_start]);
        out.push_str(&format!("{px:.0}px"));
        i = value_end;
        last_emit = i;
    }
    out.push_str(&svg[last_emit..]);
    out
}

fn ex_dimension_attr_name_len(bytes: &[u8], at: usize) -> Option<usize> {
    if at > 0 && !bytes[at - 1].is_ascii_whitespace() && bytes[at - 1] != b'<' {
        return None;
    }
    for name in [b"width".as_slice(), b"height".as_slice()] {
        let eq = at + name.len();
        if starts_with_bytes(bytes, at, name) && bytes.get(eq) == Some(&b'=') {
            return Some(name.len());
        }
    }
    None
}

fn read_ex_number_attr_value(svg: &str, value_start: usize, quote: u8) -> Option<(usize, f64)> {
    let bytes = svg.as_bytes();
    let mut value_end = value_start;
    while value_end < bytes.len() && bytes[value_end] != quote {
        value_end += 1;
    }
    if value_end >= bytes.len() {
        return None;
    }
    let value = &svg[value_start..value_end];
    let number = value.strip_suffix("ex")?;
    if number.is_empty()
        || !number
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    Some((value_end, number.parse::<f64>().ok()?))
}

fn sanitize_svg_for_zed(svg: String) -> String {
    strip_data_latex_attrs(&svg)
}

fn strip_data_latex_attrs(svg: &str) -> String {
    let bytes = svg.as_bytes();
    let mut out = String::with_capacity(svg.len());
    let mut i = 0usize;
    let mut last_emit = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace()
            && starts_with_bytes(bytes, i + 1, b"data-latex")
        {
            let attr_start = i;
            let mut j = i + 11;
            while j < bytes.len() && is_attr_name_char(bytes[j]) {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) == Some(&b'=') {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if let Some(quote @ (b'"' | b'\'')) = bytes.get(j).copied() {
                    j += 1;
                    while j < bytes.len() && bytes[j] != quote {
                        j += 1;
                    }
                    if j < bytes.len() {
                        out.push_str(&svg[last_emit..attr_start]);
                        i = j + 1;
                        last_emit = i;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    out.push_str(&svg[last_emit..]);
    out
}

fn starts_with_bytes(bytes: &[u8], at: usize, needle: &[u8]) -> bool {
    at + needle.len() <= bytes.len() && &bytes[at..at + needle.len()] == needle
}

fn is_attr_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_expands_argument() {
        let mut macros = BTreeMap::new();
        macros.insert(
            "norm".to_string(),
            MacroValue {
                body: "\\left\\lVert #1 \\right\\rVert".to_string(),
                arity: 1,
            },
        );
        assert_eq!(
            expand_macros("\\norm{x}", &macros),
            "\\left\\lVert x \\right\\rVert"
        );
    }

    #[test]
    fn finds_inline_math_region() {
        let text = "See $x^2$ now";
        let r = find_math_region(text, text.find("x").unwrap(), 2000).unwrap();
        assert_eq!(r.source, "x^2");
        assert!(!r.display);
    }

    #[test]
    fn byte_range_maps_to_actual_line() {
        // Regression for the "line 0" bug: a cite key on line 1 must
        // produce a range whose line is 1, not 0.
        let text = "first line\nsee \\cite{key} here\n";
        let brace_open = text.find('{').unwrap();
        let brace_close = text.find('}').unwrap();
        let range = byte_range_to_lsp_range(text, [brace_open + 1, brace_close]);
        assert_eq!(range.start.line, 1);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.start.character, "see \\cite{".len() as u32);
        assert_eq!(range.end.character, "see \\cite{key".len() as u32);
    }

    #[test]
    fn utf16_position_round_trips() {
        // U+1D44E (𝑎, mathematical italic a) is 2 UTF-16 code units but 1
        // char. A position after it must map back to the right byte offset.
        let text = "𝑎x";
        // offset just after '𝑎' (4 bytes) points at 'x'.
        let pos = offset_to_position(text, 4);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 2);
        assert_eq!(position_to_offset(text, pos), 4);
    }

    #[test]
    fn render_cache_hits_avoid_rerender() {
        // The cache stores post-processed SVG; a second call with identical
        // inputs must return the same string without hitting the worker.
        let renderer = MathJaxRenderer::new();
        let req = RenderRequest {
            source: "a^2 + b^2".to_string(),
            display: false,
            scale: 1.0,
            color: ColorMode::Auto,
            timeout_ms: 10_000,
        };
        let first = match renderer.render(req.clone()) {
            RenderResult::Ok { svg } => svg,
            RenderResult::Err { error } => panic!("first render failed: {error}"),
        };
        let second = match renderer.render(req) {
            RenderResult::Ok { svg } => svg,
            RenderResult::Err { error } => panic!("cached render failed: {error}"),
        };
        assert_eq!(first, second, "cached SVG must equal the first render");
    }

    #[test]
    fn render_cache_quantises_scale() {
        // 1.4 and 1.4001 must share a cache slot (quantised to 0.001).
        let cache = RenderCache::new(8);
        let k1 = RenderKey {
            source: "x".to_string(),
            display: false,
            scale_scale_milli: (1.4_f64 * 1000.0).round() as u64,
            color: ColorMode::Auto,
        };
        cache.put(k1.clone(), "<svg/>".to_string());
        let k2 = RenderKey {
            scale_scale_milli: (1.4001_f64 * 1000.0).round() as u64,
            ..k1.clone()
        };
        assert_eq!(k1.scale_scale_milli, k2.scale_scale_milli);
        assert!(cache.get(&k2).is_some(), "quantised keys must collide");
    }

    #[test]
    fn strips_mathjax_data_latex_attrs_with_raw_less_than() {
        let svg = r#"<svg><g data-latex="\displaystyle
C^{0,\gamma}(\Omega)<+\infty" data-latex-item="x<y"><path /></g></svg>"#;
        let cleaned = strip_data_latex_attrs(svg);
        assert!(!cleaned.contains("data-latex"));
        assert!(!cleaned.contains("<+\\infty"));
        assert!(cleaned.contains("<path />"));
    }

    #[test]
    fn renderer_sanitizes_formula_with_less_than() {
        let renderer = MathJaxRenderer::new();
        let result = renderer.render(RenderRequest {
            source: r"C^{0,\gamma}(\Omega):=\{f\in C(\Omega):\|f\|_{C^{0,\gamma}(\Omega)}<+\infty \},".to_string(),
            display: true,
            scale: 1.0,
            color: ColorMode::Auto,
            timeout_ms: 10_000,
        });
        let RenderResult::Ok { svg } = result else {
            panic!("expected render success");
        };
        assert!(!svg.contains("data-latex"));
        assert!(!svg.contains("<+\\infty"));
        assert!(svg.starts_with("<svg"));
    }

    #[test]
    fn rewrites_ex_dimensions_to_scaled_px() {
        let svg = r#"<svg width="10ex" height="2.5ex"><svg width="1ex" height="1ex"></svg><path stroke-width="2ex"/></svg>"#;
        let scaled = normalize_svg_size(svg.to_string(), 1.4);
        assert!(scaled.contains(r#"width="120px""#));
        assert!(scaled.contains(r#"height="36px""#));
        assert!(scaled.contains(r#"<svg width="19px" height="19px""#));
        assert!(scaled.contains(r#"stroke-width="2ex""#));
    }

    #[test]
    fn renderer_scale_changes_svg_dimensions() {
        let renderer = MathJaxRenderer::new();
        let render = |scale| match renderer.render(RenderRequest {
            source: "a+b+c".to_string(),
            display: false,
            scale,
            color: ColorMode::Auto,
            timeout_ms: 10_000,
        }) {
            RenderResult::Ok { svg } => svg,
            RenderResult::Err { error } => panic!("render failed: {error}"),
        };
        let small = render(1.0);
        let large = render(2.0);
        let small_width = root_px_attr(&small, "width").expect("small width");
        let large_width = root_px_attr(&large, "width").expect("large width");
        assert!(
            large_width > small_width * 1.7,
            "scale did not materially increase width: {small_width} -> {large_width}"
        );
    }

    fn root_px_attr(svg: &str, attr: &str) -> Option<f64> {
        let marker = format!(r#"{attr}=""#);
        let start = svg.find(&marker)? + marker.len();
        let end = svg[start..].find("px\"")? + start;
        svg[start..end].parse().ok()
    }
}
