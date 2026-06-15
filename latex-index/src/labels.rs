//! Label and reference extraction.
//!
//! Walks a LaTeX document left-to-right and emits every `\label{...}` site
//! (key + file + line + env + optional math range) into the supplied
//! `Index`.  Also exposes [`detect_ref_at`] for the cursor-context detector
//! in `cursor.rs`.
//!
//! The escape/comment/verbatim handling mirrors `server/src/scanner.ts`
//! (Section 5.1 of `docs/plan-ref-cite-hover.md`).

use crate::index::{Index, LabelEntry};
use std::path::Path;

/// Math environments whose interior is a display-math region.
pub const MATH_ENVS: &[&str] = &[
    "equation",
    "equation*",
    "align",
    "align*",
    "gather",
    "gather*",
    "multline",
    "multline*",
];

/// Theorem-like environments — these are *not* math but still get a label.
const THEOREM_ENVS: &[&str] = &[
    "theorem",
    "theorem*",
    "lemma",
    "lemma*",
    "proposition",
    "proposition*",
    "corollary",
    "corollary*",
    "definition",
    "definition*",
    "remark",
    "remark*",
    "example",
    "example*",
    "claim",
    "claim*",
    "conjecture",
    "conjecture*",
];

/// Sectioning commands whose argument is the title; `\label` immediately
/// after the argument is recorded as a `section`-env label.
const SECTION_CMDS: &[&str] = &[
    "section",
    "section*",
    "subsection",
    "subsection*",
    "subsubsection",
    "subsubsection*",
    "paragraph",
    "chapter",
    "chapter*",
];

/// `\<ref-kind>{key}` commands we route to ref hover.
pub const REF_COMMANDS: &[&str] = &[
    "ref", "eqref", "cref", "Cref", "autoref", "nameref", "pageref",
];

// ── helpers ────────────────────────────────────────────────────────────

/// True when `text[i]` is preceded by an odd number of backslashes.
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

/// Skip from `from` to just past the next `\n` (or to end of text).
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

/// Read `\begin{NAME}` / `\end{NAME}` starting at `at` (the leading `\`).
fn read_env_tag(text: &[u8], at: usize, kind: &str) -> Option<(&str, usize)> {
    let want: &[u8] = if kind == "begin" {
        b"\\begin"
    } else {
        b"\\end"
    };
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

/// Read a balanced `{…}` body starting at `at` (which must point at `{`).
/// Returns `(inner_start, inner_end, end)` where `inner_*` are the inner
/// byte offsets and `end` is just past the closing `}`.
fn read_braced(text: &[u8], at: usize) -> Option<(usize, usize, usize)> {
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

/// Read the key inside `{...}` after `\label`, returning the key string
/// and the byte offset just past the closing brace.
fn read_label_key(text: &[u8], at: usize) -> Option<(String, usize)> {
    let (a, b, end) = read_braced(text, at)?;
    let key = std::str::from_utf8(&text[a..b]).ok()?.trim().to_string();
    if key.is_empty() {
        None
    } else {
        Some((key, end))
    }
}

/// Convert a byte offset into a 0-based LSP line.  `\r` does not count
/// (matches `scanner.ts` semantics).
fn offset_to_line(text: &[u8], offset: usize) -> u32 {
    let mut line = 0u32;
    let n = offset.min(text.len());
    for &b in &text[..n] {
        if b == b'\n' {
            line += 1;
        }
    }
    line
}

fn starts_with_word(text: &[u8], at: usize, word: &[u8]) -> bool {
    at + word.len() <= text.len() && &text[at..at + word.len()] == word
}

fn next_char_ident(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'@'
}

// ── env tracking ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum EnvFrame {
    Math {
        name: String,
        body_start: usize,
        body_end: usize,
    },
    Theorem {
        name: String,
        body_start: usize,
    },
}

impl EnvFrame {
    fn name(&self) -> &str {
        match self {
            EnvFrame::Math { name, .. } | EnvFrame::Theorem { name, .. } => name,
        }
    }
    fn is_math(&self) -> bool {
        matches!(self, EnvFrame::Math { .. })
    }
}

fn current_env_label(stack: &[EnvFrame]) -> String {
    stack
        .last()
        .map(|f| f.name().to_string())
        .unwrap_or_else(|| "document".to_string())
}

fn current_math_range(stack: &[EnvFrame], label_at: usize) -> Option<[usize; 2]> {
    for frame in stack.iter().rev() {
        if let EnvFrame::Math {
            body_start,
            body_end,
            ..
        } = frame
        {
            if *body_end > *body_start && label_at >= *body_start && label_at <= *body_end {
                return Some([*body_start, *body_end]);
            }
        }
    }
    None
}

// ── public: extraction ─────────────────────────────────────────────────

/// Parse `text` (contents of a `.tex` file at `path`) and insert every
/// `\label{...}` site into `index`.  Replaces any prior entries from `path`.
pub fn extract_labels(text: &str, path: &Path, index: &Index) {
    let bytes = text.as_bytes();
    index.labels.retain(|_, v| v.file != path);

    let mut env_stack: Vec<EnvFrame> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];

        // 1. comment
        if b == b'%' && !is_escaped(bytes, i) {
            i = skip_to_eol(bytes, i + 1);
            continue;
        }

        // 2. \begin{...}
        if b == b'\\' && starts_with_word(bytes, i, b"\\begin") && !is_escaped(bytes, i) {
            if let Some((name, tag_end)) = read_env_tag(bytes, i, "begin") {
                if MATH_ENVS.contains(&name) {
                    env_stack.push(EnvFrame::Math {
                        name: name.to_string(),
                        body_start: tag_end,
                        body_end: 0,
                    });
                    i = tag_end;
                    continue;
                } else if THEOREM_ENVS.contains(&name) {
                    env_stack.push(EnvFrame::Theorem {
                        name: name.to_string(),
                        body_start: tag_end,
                    });
                    i = tag_end;
                    continue;
                }
            }
        }

        // 3. \end{...}
        if b == b'\\' && starts_with_word(bytes, i, b"\\end") && !is_escaped(bytes, i) {
            if let Some((name, _)) = read_env_tag(bytes, i, "end") {
                if let Some(pos) = env_stack.iter().rposition(|f| f.name() == name) {
                    if env_stack[pos].is_math() {
                        if let EnvFrame::Math { body_end, .. } = &mut env_stack[pos] {
                            *body_end = i;
                        }
                    }
                    env_stack.remove(pos);
                }
                // Skip past `\end{name}` tag.
                let tag_len = b"\\end".len() + 1 + name.len() + 1;
                i += tag_len;
                continue;
            }
        }

        // 4. \label{...}
        if b == b'\\'
            && starts_with_word(bytes, i, b"\\label")
            && !is_escaped(bytes, i)
            && !next_char_ident(*bytes.get(i + b"\\label".len()).unwrap_or(&0))
        {
            let mut probe = i + b"\\label".len();
            while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
                probe += 1;
            }
            if bytes.get(probe) == Some(&b'{') {
                if let Some((key, end)) = read_label_key(bytes, probe) {
                    let line = offset_to_line(bytes, probe);
                    let env = current_env_label(&env_stack);
                    let math = current_math_range(&env_stack, probe);
                    let caption = best_caption_for_env(&env, &env_stack, bytes, probe);
                    let entry = LabelEntry {
                        key,
                        file: path.to_path_buf(),
                        offset: i,
                        line,
                        env,
                        math,
                        caption,
                    };
                    index.labels.insert(entry.key.clone(), entry);
                    i = end;
                    continue;
                }
            }
        }

        // 5. \section{...}\label{...}
        if b == b'\\' && !is_escaped(bytes, i) {
            for cmd in SECTION_CMDS {
                let w = format!("\\{}", cmd);
                let wb = w.as_bytes();
                if starts_with_word(bytes, i, wb)
                    && !next_char_ident(*bytes.get(i + wb.len()).unwrap_or(&0))
                {
                    let mut probe = i + wb.len();
                    while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
                        probe += 1;
                    }
                    if let Some((title_start, title_end, _e)) = read_braced(bytes, probe) {
                        let mut p2 = title_end + 1;
                        while p2 < bytes.len() && bytes[p2].is_ascii_whitespace() {
                            p2 += 1;
                        }
                        if starts_with_word(bytes, p2, b"\\label") {
                            let after_label = p2 + b"\\label".len();
                            let mut p3 = after_label;
                            while p3 < bytes.len() && (bytes[p3] == b' ' || bytes[p3] == b'\t') {
                                p3 += 1;
                            }
                            if bytes.get(p3) == Some(&b'{') {
                                if let Some((key, _)) = read_label_key(bytes, p3) {
                                    let line = offset_to_line(bytes, p2);
                                    let caption = std::str::from_utf8(&bytes[title_start..title_end])
                                        .unwrap_or("")
                                        .trim()
                                        .to_string();
                                    let entry = LabelEntry {
                                        key,
                                        file: path.to_path_buf(),
                                        offset: p2,
                                        line,
                                        env: "section".to_string(),
                                        math: None,
                                        caption,
                                    };
                                    index.labels.insert(entry.key.clone(), entry);
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }

        i += 1;
    }
}

fn best_caption_for_env(
    env: &str,
    stack: &[EnvFrame],
    bytes: &[u8],
    label_at: usize,
) -> String {
    if THEOREM_ENVS.iter().any(|e| *e == env) {
        if let Some(EnvFrame::Theorem { body_start, .. }) = stack.last() {
            let slice_start = *body_start;
            let slice_end = label_at.min(bytes.len());
            let s = std::str::from_utf8(&bytes[slice_start..slice_end]).unwrap_or("");
            for line in s.split('\n') {
                let t = line.trim();
                if !t.is_empty() {
                    return t.chars().take(120).collect();
                }
            }
        }
    }
    String::new()
}

// ── public: cursor-side ref detection ──────────────────────────────────

/// Detect whether `text[offset..]` is inside a `\<ref-kind>{<key>}` argument.
/// Walks backwards from `offset` to find the nearest matching command whose
/// opening brace encloses the cursor.  Returns the citation key and the
/// byte range of the key (both endpoints exclusive of the braces).
pub fn detect_ref_at(text: &str, offset: usize) -> Option<(String, [usize; 2])> {
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
                    return match_key_after_command(bytes, i);
                }
                depth -= 1;
            }
            b'\\' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

fn match_key_after_command(bytes: &[u8], brace_open: usize) -> Option<(String, [usize; 2])> {
    let mut end = brace_open;
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
    if !REF_COMMANDS.contains(&cmd) {
        return None;
    }
    let (kstart, kend, _) = read_braced(bytes, brace_open)?;
    let key = std::str::from_utf8(&bytes[kstart..kend]).ok()?.trim().to_string();
    Some((key, [kstart, kend]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_in_equation() {
        let idx = Index::new();
        let text = r#"\begin{equation}
a^2 + b^2 = c^2 \label{eq:pythag}
\end{equation}"#;
        extract_labels(text, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("eq:pythag").expect("label found");
        assert_eq!(entry.env, "equation");
        assert!(entry.math.is_some());
    }

    #[test]
    fn label_in_section() {
        let idx = Index::new();
        let text = r#"\section{Introduction}\label{sec:intro}
body"#;
        extract_labels(text, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("sec:intro").expect("label found");
        assert_eq!(entry.env, "section");
        assert_eq!(entry.caption, "Introduction");
    }

    #[test]
    fn nested_env_labels() {
        let idx = Index::new();
        let text = r#"\begin{align}
\begin{equation}
x = 1 \label{eq:inner}
\end{equation}
\end{align}"#;
        extract_labels(text, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("eq:inner").expect("found");
        assert_eq!(entry.env, "equation");
    }

    #[test]
    fn ref_at_cursor() {
        let text = r#"See \ref{eq:foo} for details."#;
        // position is just before `e` in `eq:foo`.
        let pos = text.find("eq:foo").unwrap();
        let (key, range) = detect_ref_at(text, pos + 1).unwrap();
        assert_eq!(key, "eq:foo");
        assert_eq!(range[0], text.find('{').unwrap() + 1);
        assert_eq!(range[1], text.find('}').unwrap());
    }

    #[test]
    fn ref_at_cursor_inside_crefs() {
        let text = r#"\cref{eq:x,y:z}"#;
        let pos = text.find("eq:x").unwrap() + 1;
        let (key, _) = detect_ref_at(text, pos).unwrap();
        assert_eq!(key, "eq:x,y:z");
    }
}
