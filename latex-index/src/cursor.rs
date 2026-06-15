//! Cursor-context detection.
//!
//! Given `(uri, offset)` and the in-memory text for `uri` (placed there by
//! the most recent `update_file` call), decide which kind of token the
//! cursor is on:
//!
//!   * `cite`  — inside a `\cite{…}` argument
//!   * `ref`   — inside a `\ref`/`\eqref`/`\cref`/… argument
//!   * `math`  — inside any math region (inline `$…$`, `\(…\)`, `\[…\]`,
//!               `$$…$$`, or a `equation`/`align`/… environment)
//!   * `none`  — none of the above
//!
//! The math scanner is a hand-rolled port of `server/src/scanner.ts`.  See
//! `docs/plan-ref-cite-hover.md` Section 7.

use crate::labels::detect_ref_at;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorContext {
    /// `"cite" | "ref" | "math" | "none"`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<[usize; 2]>,
}

impl CursorContext {
    pub fn none() -> Self {
        Self {
            kind: "none".to_string(),
            key: None,
            range: None,
        }
    }
}

/// In-memory cache of the most recent buffer text per URI.  Cheap clone
/// (Arc-backed); the lock is held only for short HashMap operations.
#[derive(Debug, Default, Clone)]
pub struct BufferStore {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

impl BufferStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, uri: String, text: String) {
        let mut g = self.inner.lock().expect("buffer-store poisoned");
        g.insert(uri, text);
    }

    pub fn get(&self, uri: &str) -> Option<String> {
        let g = self.inner.lock().expect("buffer-store poisoned");
        g.get(uri).cloned()
    }

    pub fn close(&self, uri: &str) {
        let mut g = self.inner.lock().expect("buffer-store poisoned");
        g.remove(uri);
    }
}

// ── cite/ref detectors ────────────────────────────────────────────────

const CITE_COMMANDS: &[&str] = &[
    "cite", "Cite", "citep", "citep*", "citet", "citet*", "citeauthor", "citeyear",
    "citeyearpar", "autocite", "autocite*", "parencite", "footcite", "textcite",
    "smartcite", "supercite",
];

/// Detect whether `text[offset..]` is inside a `\cite{...}` argument.
pub fn detect_cite_at(text: &str, offset: usize) -> Option<(String, [usize; 2])> {
    detect_braced_command_at(text, offset, CITE_COMMANDS)
}

/// Like `detect_cite_at` but for any of the recognised ref-commands.
pub fn detect_ref_command_at(text: &str, offset: usize) -> Option<(String, [usize; 2])> {
    detect_braced_command_at(text, offset, crate::labels::REF_COMMANDS)
}

/// Generic brace-balanced detector: walk back from `offset`, find the
/// opening `{` of the cursor's enclosing brace pair, and check whether the
/// immediately preceding command name is one of `commands`.
fn detect_braced_command_at(
    text: &str,
    offset: usize,
    commands: &[&str],
) -> Option<(String, [usize; 2])> {
    let bytes = text.as_bytes();
    let off = offset.min(bytes.len());

    let mut depth = 0i32;
    let mut i = off;
    while i > 0 {
        i -= 1;
        let b = bytes[i];
        match b {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    // Walk back over the command name (alphabetic chars).
                    let mut end = i;
                    while end > 0 && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t') {
                        end -= 1;
                    }
                    let cmd_end = end;
                    let mut start = end;
                    while start > 0 && bytes[start - 1].is_ascii_alphabetic() {
                        start -= 1;
                    }
                    if start == 0 || bytes[start - 1] != b'\\' {
                        return None;
                    }
                    let cmd = std::str::from_utf8(&bytes[start..cmd_end]).ok()?;
                    if !commands.contains(&cmd) {
                        return None;
                    }
                    // Capture the key inside `{…}` (skip outer braces).
                    if let Some((a, b, _)) = read_braced_simple(bytes, i) {
                        let key = std::str::from_utf8(&bytes[a..b]).ok()?.trim().to_string();
                        return Some((key, [a, b]));
                    }
                    return None;
                }
                depth -= 1;
            }
            b'\\' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

fn read_braced_simple(text: &[u8], at: usize) -> Option<(usize, usize, usize)> {
    if text.get(at) != Some(&b'{') {
        return None;
    }
    let mut depth = 1i32;
    let mut i = at + 1;
    while i < text.len() {
        match text[i] {
            b'\\' => {
                i = (i + 2).min(text.len());
                continue;
            }
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

// ── public: cursor_context ─────────────────────────────────────────────

pub fn cursor_context(uri: &str, offset: usize, store: &BufferStore) -> CursorContext {
    let Some(text) = store.get(uri) else {
        return CursorContext::none();
    };

    if let Some((key, range)) = detect_cite_at(&text, offset) {
        return CursorContext {
            kind: "cite".to_string(),
            key: Some(key),
            range: Some(range),
        };
    }
    if let Some((key, range)) = detect_ref_command_at(&text, offset) {
        return CursorContext {
            kind: "ref".to_string(),
            key: Some(key),
            range: Some(range),
        };
    }
    if let Some((key, range)) = detect_ref_at(&text, offset) {
        return CursorContext {
            kind: "ref".to_string(),
            key: Some(key),
            range: Some(range),
        };
    }
    if find_math_at(&text, offset).is_some() {
        return CursorContext {
            kind: "math".to_string(),
            key: None,
            range: None,
        };
    }
    CursorContext::none()
}

// ── math scanner (port of scanner.ts) ──────────────────────────────────

#[derive(Debug, Clone)]
struct MathSpan {
    start: usize,
    end: usize,
    display: bool,
}

const MATH_ENVS: &[&str] = &[
    "equation", "equation*", "align", "align*", "gather", "gather*", "multline", "multline*",
];

const VERBATIM_ENVS: &[&str] = &["verbatim", "lstlisting", "minted"];

fn is_escaped(text: &[u8], i: usize) -> bool {
    let mut bs = 0;
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

fn find_dollar_closer(text: &[u8], from: usize, want_double: bool) -> usize {
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
            if want_double {
                if is_double {
                    return i;
                }
            } else if !is_double {
                return i;
            }
        }
        i += 1;
    }
    text.len()
}

fn read_env_tag(text: &[u8], at: usize, kind: &str) -> Option<(&str, usize)> {
    let want: &[u8] = if kind == "begin" { b"\\begin" } else { b"\\end" };
    if text.len() < at + want.len() || &text[at..at + want.len()] != want {
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
    let name = std::str::from_utf8(&text[start..i]).ok()?;
    Some((name, i + 1))
}

fn tokenize_math(text: &[u8]) -> Vec<MathSpan> {
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        let b = text[i];
        if b == b'%' && !is_escaped(text, i) {
            i = skip_to_eol(text, i + 1);
            continue;
        }
        if b == b'\\' && text.len() >= i + 6 && &text[i..i + 6] == b"\\begin" && !is_escaped(text, i) {
            if let Some((name, tag_end)) = read_env_tag(text, i, "begin") {
                let close_tag = format!("\\end{{{}}}", name);
                let cb = close_tag.as_bytes();
                let close = find_subslice(text, tag_end, cb);
                let end_delim = close.map(|p| p + cb.len()).unwrap_or(text.len());
                if VERBATIM_ENVS.contains(&name) {
                    i = end_delim;
                    continue;
                }
                if MATH_ENVS.contains(&name) {
                    spans.push(MathSpan {
                        start: i,
                        end: end_delim,
                        display: true,
                    });
                    i = end_delim;
                    continue;
                }
            }
        }
        if b == b'$' && !is_escaped(text, i) {
            let is_double = text.get(i + 1) == Some(&b'$');
            let w = if is_double { 2 } else { 1 };
            let close = find_dollar_closer(text, i + w, is_double);
            let end = if close < text.len() { close + w } else { text.len() };
            spans.push(MathSpan { start: i, end, display: is_double });
            i = end;
            continue;
        }
        if b == b'\\' && !is_escaped(text, i) {
            let next = text.get(i + 1).copied();
            if next == Some(b'(') || next == Some(b'[') {
                let display = next == Some(b'[');
                let close_delim: &[u8] = if display { b"\\]" } else { b"\\)" };
                let close = find_subslice(text, i + 2, close_delim);
                let end = close.map(|p| p + close_delim.len()).unwrap_or(text.len());
                spans.push(MathSpan { start: i, end, display });
                i = end;
                continue;
            }
        }
        i += 1;
    }
    spans
}

fn find_subslice(text: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= text.len() {
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

fn find_math_at(text: &str, offset: usize) -> Option<MathSpan> {
    let bytes = text.as_bytes();
    let off = offset.min(bytes.len());
    for span in tokenize_math(bytes) {
        if off >= span.start && off < span.end {
            return Some(span);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cite_inside_braces() {
        let text = r#"See \cite{einstein1905}."#;
        let pos = text.find("einstein1905").unwrap() + 1;
        let (k, _) = detect_cite_at(text, pos).unwrap();
        assert_eq!(k, "einstein1905");
    }

    #[test]
    fn detects_ref_inside_braces() {
        let text = r#"\eqref{eq:x}"#;
        let pos = text.find("eq:x").unwrap() + 1;
        let (k, _) = detect_ref_command_at(text, pos).unwrap();
        assert_eq!(k, "eq:x");
    }

    #[test]
    fn cursor_context_returns_math_for_inline() {
        let store = BufferStore::new();
        let uri = "file:///a.tex".to_string();
        let text = r#"Inline $a^2 + b^2 = c^2$ math."#.to_string();
        store.put(uri.clone(), text.clone());
        let pos = text.find("a^2").unwrap() + 1;
        let ctx = cursor_context(&uri, pos, &store);
        assert_eq!(ctx.kind, "math");
    }

    #[test]
    fn cursor_context_returns_none_outside() {
        let store = BufferStore::new();
        let uri = "file:///a.tex".to_string();
        store.put(uri.clone(), "Hello world.".to_string());
        let ctx = cursor_context(&uri, 3, &store);
        assert_eq!(ctx.kind, "none");
    }
}
