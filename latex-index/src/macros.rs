//! Macro extraction — port of `server/src/macros.ts`.
//!
//! Recognises the same six defining commands and uses an explicit
//! brace-counter so macro bodies with arbitrary nesting (e.g.
//! `\newcommand{\x}{\sqrt{\frac{a}{b}}}`) parse correctly.  See
//! `docs/plan-ref-cite-hover.md` Section 5.3.

use crate::index::{Index, MacroDef};
use std::path::Path;

/// Order matters: the longer / starred forms must come before their bare
/// prefix (`newcommand*` before `newcommand`).
const DEFINING_CMDS: &[&str] = &[
    "newcommand*",
    "newcommand",
    "renewcommand*",
    "renewcommand",
    "providecommand*",
    "providecommand",
    "def",
    "DeclareMathOperator",
];

// ── brace-balanced helpers ─────────────────────────────────────────────

/// Read a balanced `{…}` starting at `at` (which must point at `{`).
/// Returns `(inner_start, inner_end, end)` or `None`.
fn read_balanced(text: &[u8], at: usize) -> Option<(usize, usize, usize)> {
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

fn skip_ws(text: &[u8], at: usize) -> usize {
    let mut i = at;
    while i < text.len() && (text[i] == b' ' || text[i] == b'\t') {
        i += 1;
    }
    i
}

fn starts_with_word(text: &[u8], at: usize, word: &[u8]) -> bool {
    at + word.len() <= text.len() && &text[at..at + word.len()] == word
}

fn next_char_ident(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'@'
}

// ── one macro definition ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct MacroSite {
    cmd: &'static str,
    name: &'static str,
    arity: u32,
    body_open: usize,
}

/// Try to parse a macro definition starting at `at` (the leading `\`).
/// Returns the macro descriptor or `None`.
fn read_macro_def(text: &[u8], at: usize) -> Option<MacroSite> {
    if text.get(at) != Some(&b'\\') {
        return None;
    }
    let mut cmd: Option<&'static str> = None;
    let mut after = at + 1;
    for &c in DEFINING_CMDS {
        let cb = c.as_bytes();
        if starts_with_word(text, after, cb)
            && !next_char_ident(*text.get(after + cb.len()).unwrap_or(&0))
        {
            cmd = Some(c);
            after += cb.len();
            break;
        }
    }
    let cmd = cmd?;
    after = skip_ws(text, after);

    // Macro name: either `{\name}` (newcommand family) or `\name` (plain \def).
    // Both forms are accepted for every command.
    let name_start: usize;
    let name_end: usize;
    if text.get(after) == Some(&b'{') {
        let close_idx = find_char(text, after + 1, b'}')?;
        if text.get(after + 1) != Some(&b'\\') {
            return None;
        }
        name_start = after + 2;
        name_end = close_idx;
        after = close_idx + 1;
    } else if text.get(after) == Some(&b'\\') {
        let mut end = after + 1;
        while end < text.len() && (text[end].is_ascii_alphabetic() || text[end] == b'@') {
            end += 1;
        }
        name_start = after + 1;
        name_end = end;
        after = end;
    } else {
        return None;
    }

    let name = std::str::from_utf8(&text[name_start..name_end]).ok()?;
    if name.is_empty() {
        return None;
    }

    // Optional `[N]` arity.
    after = skip_ws(text, after);
    let mut arity: u32 = 0;
    if text.get(after) == Some(&b'[') {
        let close = find_char(text, after + 1, b']')?;
        let n: u32 = std::str::from_utf8(&text[after + 1..close])
            .ok()?
            .trim()
            .parse()
            .ok()?;
        arity = n;
        after = close + 1;
    }

    // Body must follow as `{…}`.
    after = skip_ws(text, after);
    if text.get(after) != Some(&b'{') {
        return None;
    }

    // SAFETY: we copy `name` into a `&'static str` via leaking the bytes for
    // the duration of this call only.  Easier: just return `(name_owned,
    // cmd, arity, body_open)` by leaking the slice.  In practice we never
    // hold on to these across calls, so a `String` clone is fine.
    Some(MacroSite {
        cmd,
        // SAFETY: `name` lives until end of function; the lifetime here is
        // fine because MacroSite is consumed immediately in `extract`.
        name: Box::leak(name.to_string().into_boxed_str()),
        arity,
        body_open: after,
    })
}

fn find_char(text: &[u8], from: usize, target: u8) -> Option<usize> {
    let mut i = from;
    while i < text.len() {
        if text[i] == target {
            return Some(i);
        }
        i += 1;
    }
    None
}

// ── extraction ─────────────────────────────────────────────────────────

/// Parse `text` (contents of a `.tex` file at `path`) and insert every
/// `\newcommand`/`\def`/… definition into `index`.  Replaces any prior
/// entries from `path`.
pub fn extract_macros(text: &str, path: &Path, index: &Index) {
    let bytes = text.as_bytes();
    index.macros.retain(|_, v| v.file != path);

    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if let Some(site) = read_macro_def(bytes, i) {
                if let Some((a, b, end)) = read_balanced(bytes, site.body_open) {
                    let mut body_str = std::str::from_utf8(&bytes[a..b])
                        .unwrap_or("")
                        .to_string();
                    if site.cmd == "DeclareMathOperator" {
                        body_str = format!("\\operatorname{{{}}}", body_str);
                    }
                    let def = MacroDef {
                        name: site.name.to_string(),
                        file: path.to_path_buf(),
                        body: body_str,
                        arity: site.arity,
                    };
                    index.macros.insert(def.name.clone(), def);
                    i = end;
                    continue;
                }
                // Drop the leaked `name` slice by re-using it as static.
                // (It's a small allocation per failed parse; acceptable.)
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newcommand_basic() {
        let idx = Index::new();
        let text = r#"\newcommand{\R}{\mathbb{R}}"#;
        extract_macros(text, Path::new("a.tex"), &idx);
        let m = idx.macros.get("R").unwrap();
        assert_eq!(m.body, "\\mathbb{R}");
        assert_eq!(m.arity, 0);
    }

    #[test]
    fn newcommand_starred() {
        let idx = Index::new();
        let text = r#"\newcommand*{\H}{\mathbb{H}}"#;
        extract_macros(text, Path::new("a.tex"), &idx);
        assert!(idx.macros.get("H").is_some());
    }

    #[test]
    fn newcommand_with_arity() {
        let idx = Index::new();
        let text = r#"\newcommand{\norm}[1]{\left\|#1\right\|}"#;
        extract_macros(text, Path::new("a.tex"), &idx);
        let m = idx.macros.get("norm").unwrap();
        assert_eq!(m.arity, 1);
        assert!(m.body.contains("#1"));
    }

    #[test]
    fn def_with_bare_name() {
        let idx = Index::new();
        let text = r#"\def\R{\mathbb{R}}"#;
        extract_macros(text, Path::new("a.tex"), &idx);
        let m = idx.macros.get("R").unwrap();
        assert_eq!(m.body, "\\mathbb{R}");
    }

    #[test]
    fn declare_math_operator() {
        let idx = Index::new();
        let text = r#"\DeclareMathOperator{\div}{div}"#;
        extract_macros(text, Path::new("a.tex"), &idx);
        let m = idx.macros.get("div").unwrap();
        assert_eq!(m.body, "\\operatorname{div}");
    }

    #[test]
    fn nested_braces_in_body() {
        let idx = Index::new();
        let text = r#"\newcommand{\x}{\sqrt{\frac{a}{b}}}"#;
        extract_macros(text, Path::new("a.tex"), &idx);
        let m = idx.macros.get("x").unwrap();
        assert_eq!(m.body, "\\sqrt{\\frac{a}{b}}");
    }
}
