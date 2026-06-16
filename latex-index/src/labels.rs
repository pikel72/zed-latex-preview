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
fn read_env_tag<'a>(text: &'a [u8], at: usize, kind: &str) -> Option<(&'a str, usize)> {
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

// ── snippet extraction (Phase 2 §4.1) ──────────────────────────────────

/// Line and byte-size limits for the snippet per spec §4.1.
const SNIPPET_MAX_LINES: usize = 12;
const SNIPPET_MAX_BYTES: usize = 4 * 1024;

/// Truncation marker appended when the body exceeds limits.
const TRUNCATED_MARKER: &str = "% (truncated)";

/// Find the start of the line containing `offset` (the byte just past the
/// preceding `\n`, or 0).
fn line_start(text: &[u8], offset: usize) -> usize {
    let n = offset.min(text.len());
    let mut i = n;
    while i > 0 && text[i - 1] != b'\n' {
        i -= 1;
    }
    i
}

/// Find the start of the line after the line containing `offset` (the byte
/// just past the next `\n`).
fn line_end(text: &[u8], offset: usize) -> usize {
    skip_to_eol(text, offset.min(text.len()))
}

/// Trim leading and trailing blank lines from `s` (blank = empty or
/// whitespace-only).
fn trim_blank_lines(s: &str) -> &str {
    let mut start = 0;
    let bytes = s.as_bytes();
    while start < bytes.len() {
        let line_end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| start + p)
            .unwrap_or(bytes.len());
        let line = &bytes[start..line_end];
        if line.iter().any(|b| !b.is_ascii_whitespace()) {
            break;
        }
        start = if line_end < bytes.len() { line_end + 1 } else { bytes.len() };
    }
    let mut end = bytes.len();
    while end > start {
        let line_start = bytes[..end]
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        let line = &bytes[line_start..end];
        if line.iter().any(|b| !b.is_ascii_whitespace()) {
            break;
        }
        end = line_start;
        if end == 0 {
            break;
        }
        // step past the trailing '\n'
        if end > 0 && bytes[end - 1] == b'\n' {
            end -= 1;
        }
    }
    std::str::from_utf8(&bytes[start..end]).unwrap_or("")
}

/// Apply the §4.1 truncation policy to a candidate body string.
fn truncate_snippet(body: &str) -> String {
    let total_lines = body.matches('\n').count() + if body.ends_with('\n') { 0 } else { 1 };
    let byte_len = body.len();
    if total_lines <= SNIPPET_MAX_LINES && byte_len <= SNIPPET_MAX_BYTES {
        return body.to_string();
    }
    let mut kept = String::new();
    let mut count = 0usize;
    let mut consumed = 0usize;
    for line in body.split_inclusive('\n') {
        let prospective = consumed + line.len();
        if count + 1 > SNIPPET_MAX_LINES || prospective > SNIPPET_MAX_BYTES {
            break;
        }
        kept.push_str(line);
        consumed = prospective;
        count += 1;
    }
    if !kept.ends_with('\n') && !kept.is_empty() {
        kept.push('\n');
    }
    kept.push_str(TRUNCATED_MARKER);
    kept
}

/// Build the snippet for a label inside a math or theorem env.
/// Uses the env's recorded body range; falls back to `"\label{...}"` line
/// with the truncation marker when no body boundary can be located.
fn snippet_for_env_body(
    text: &[u8],
    label_at: usize,
    body_start: Option<usize>,
    body_end: Option<usize>,
    section_at: usize,
) -> String {
    let label_line_start = line_start(text, label_at);
    let label_line_end = line_end(text, label_at);

    // No body boundary → fallback to the single \label line + truncated.
    let (Some(bs), Some(be)) = (body_start, body_end) else {
        let line = &text[label_line_start..label_line_end];
        let line = std::str::from_utf8(line).unwrap_or("");
        let mut out = line.trim_end_matches('\n').to_string();
        out.push('\n');
        out.push_str(TRUNCATED_MARKER);
        return out;
    };

    if be <= bs {
        // Unclosed env — same fallback.
        let line = &text[label_line_start..label_line_end];
        let line = std::str::from_utf8(line).unwrap_or("");
        let mut out = line.trim_end_matches('\n').to_string();
        out.push('\n');
        out.push_str(TRUNCATED_MARKER);
        return out;
    }

    let bs = bs.min(text.len());
    let be = be.min(text.len());
    let raw = &text[bs..be];
    let raw_str = std::str::from_utf8(raw).unwrap_or("");
    let trimmed = trim_blank_lines(raw_str);
    let _ = section_at; // currently unused but kept for future call sites
    truncate_snippet(trimmed)
}

/// Build the snippet for `\section{...}\label{...}`: the single line
/// containing the section command, with the trailing `\label{...}`.
///
/// When `\section` and `\label` live on the same line, that's one line.
/// When they live on different lines, we deliberately take **only the
/// section line** — the `\label` line gets surfaced via the file:line
/// pointer in the hover, not duplicated as snippet content.
fn snippet_for_section(text: &[u8], section_at: usize, label_at: usize) -> String {
    let ls = line_start(text, section_at);
    // Always end the snippet at the section line, never at the label line.
    // `label_at` is passed for symmetry with `snippet_for_env_body`; the
    // section-line snippet does not need it.
    let _ = label_at;
    let le = line_end(text, section_at);
    let line = &text[ls..le];
    let s = std::str::from_utf8(line).unwrap_or("");
    truncate_snippet(s.trim_end_matches('\n'))
}

/// Build the snippet for a free-floating `\label{...}`: the line containing
/// `\label` plus one line of context before (≤ 2 lines total).
fn snippet_for_free_floating(text: &[u8], label_at: usize) -> String {
    let ls = line_start(text, label_at);
    // One line of context before, if available.
    let prev_le = ls;
    let prev_ls = if prev_le > 0 {
        // Find the end of the previous line.
        let mut i = prev_le - 1;
        // skip the '\n' separator
        if text[i] == b'\n' {
            i -= 1;
        }
        line_start(text, i)
    } else {
        prev_le
    };
    let le = line_end(text, label_at);
    let s = &text[prev_ls..le];
    let out = std::str::from_utf8(s).unwrap_or("");
    truncate_snippet(out.trim_end_matches('\n'))
}

/// Locate the matching `\end{<env_name>}` for a math frame whose `body_end`
/// is still 0 (the env hasn't been closed yet — the label is processed
/// before `\end`).  Returns the byte offset of the leading `\` of `\end{…}`,
/// or `None` if no match can be found (unclosed env).
fn find_math_end(text: &[u8], env_name: &str, search_from: usize) -> Option<usize> {
    let needle = format!("\\end{{{env_name}}}");
    let n = needle.as_bytes();
    let mut i = search_from;
    while i + n.len() <= text.len() {
        if starts_with_word(text, i, n) && !is_escaped(text, i) {
            return Some(i);
        }
        // Skip comments quickly to mirror the main loop's comment handling.
        if text[i] == b'%' && !is_escaped(text, i) {
            i = skip_to_eol(text, i + 1);
            continue;
        }
        i += 1;
    }
    None
}

/// Dispatch to the correct snippet extraction for the current env stack at
/// the time the `\label` is processed.
fn compute_snippet_for_label(env_stack: &[EnvFrame], text: &[u8], label_at: usize) -> String {
    // Look at the innermost enclosing frame, if any.
    if let Some(frame) = env_stack.last() {
        match frame {
            EnvFrame::Math {
                body_start,
                body_end,
                name,
            } => {
                // `body_end` is 0 until `\end{…}` is processed; the label
                // sits inside the body, so we often hit this branch with
                // `body_end == 0`.  Resolve by scanning forward for the
                // matching `\end{name}` from `body_start`.
                let resolved_end = if *body_end == 0 {
                    find_math_end(text, name, *body_start).unwrap_or(0)
                } else {
                    *body_end
                };
                let (bs, be) = if resolved_end > *body_start {
                    (Some(*body_start), Some(resolved_end))
                } else {
                    (None, None)
                };
                return snippet_for_env_body(text, label_at, bs, be, label_at);
            }
            EnvFrame::Theorem { body_start, .. } => {
                // For theorem envs we don't track body_end (the env closes
                // at the matching \end{...}); use the label position as the
                // upper bound — extract up to the label, which is what we
                // want anyway for the first-line caption.
                return snippet_for_env_body(
                    text,
                    label_at,
                    Some(*body_start),
                    Some(label_at),
                    label_at,
                );
            }
        }
    }
    // Free-floating \label.
    snippet_for_free_floating(text, label_at)
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
            // body_end may be 0 when the env hasn't closed yet (label
            // inside the body is processed before `\end{...}`).
            if label_at >= *body_start && (*body_end == 0 || label_at <= *body_end) {
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
                    let snippet = compute_snippet_for_label(&env_stack, bytes, i);
                    let entry = LabelEntry {
                        key,
                        file: path.to_path_buf(),
                        offset: i,
                        line,
                        env,
                        math,
                        caption,
                        snippet,
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
                                if let Some((key, label_end)) = read_label_key(bytes, p3) {
                                    let line = offset_to_line(bytes, p2);
                                    let caption = std::str::from_utf8(&bytes[title_start..title_end])
                                        .unwrap_or("")
                                        .trim()
                                        .to_string();
                                    let snippet = snippet_for_section(bytes, i, p2);
                                    let entry = LabelEntry {
                                        key,
                                        file: path.to_path_buf(),
                                        offset: p2,
                                        line,
                                        env: "section".to_string(),
                                        math: None,
                                        caption,
                                        snippet,
                                    };
                                    index.labels.insert(entry.key.clone(), entry);
                                    // Advance past the label so step 4 doesn't
                                    // re-insert it with env="document".
                                    i = label_end - 1; // -1 compensates for i+=1 below
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

    // ── Phase 2 §7.1: snippet extraction unit tests ─────────────────────

    #[test]
    fn snippet_for_equation_includes_body() {
        let idx = Index::new();
        let text = "\
\\begin{equation}
a^2 + b^2 = c^2 \\label{eq:pythag}
\\end{equation}";
        extract_labels(text, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("eq:pythag").expect("label found");
        let snippet = &entry.snippet;
        // Body is between \\begin{equation} and \\end{equation}.
        assert!(snippet.contains("a^2 + b^2 = c^2"), "snippet = {snippet:?}");
        // No truncation marker.
        assert!(!snippet.contains("% (truncated)"));
    }

    #[test]
    fn snippet_for_theorem_captures_first_line() {
        let idx = Index::new();
        let text = "\
\\begin{theorem}
Pythagoras: a squared plus b squared equals c squared.
The proof is left as an exercise. \\label{thm:pythag}
\\end{theorem}";
        extract_labels(text, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("thm:pythag").expect("label found");
        let snippet = &entry.snippet;
        // First line of body captured.
        assert!(
            snippet.contains("Pythagoras: a squared plus b squared equals c squared."),
            "snippet = {snippet:?}"
        );
        // No truncation marker.
        assert!(!snippet.contains("% (truncated)"));
    }

    #[test]
    fn snippet_for_section_is_single_line() {
        let idx = Index::new();
        let text = "\\section{Introduction}\\label{sec:intro}\nbody paragraph here.";
        extract_labels(text, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("sec:intro").expect("label found");
        let snippet = &entry.snippet;
        // The single line containing the \\section command + trailing \\label.
        assert!(snippet.contains("\\section{Introduction}"));
        assert!(snippet.contains("\\label{sec:intro}"));
        // Must not contain the body line.
        assert!(!snippet.contains("body paragraph here."));
    }

    #[test]
    fn snippet_truncated_at_12_lines() {
        let idx = Index::new();
        // Build a theorem body with 20 distinct lines so we exceed 12.
        let mut body = String::from("\\begin{theorem}\n");
        for n in 0..20 {
            body.push_str(&format!("line {n}\n"));
        }
        body.push_str("\\label{thm:long}\n\\end{theorem}");
        extract_labels(&body, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("thm:long").expect("label found");
        let snippet = &entry.snippet;
        // Truncation marker present.
        assert!(snippet.contains("% (truncated)"), "snippet = {snippet:?}");
        // ≤ 12 lines of body + the marker line.
        let line_count = snippet.lines().count();
        assert!(
            line_count <= 13,
            "expected ≤ 13 lines, got {line_count}: {snippet:?}"
        );
        // First line preserved.
        assert!(snippet.contains("line 0"), "snippet = {snippet:?}");
        // Last kept body line must not be `line 19`.
        assert!(
            !snippet.contains("line 19"),
            "snippet should not include line 19: {snippet:?}"
        );
    }

    #[test]
    fn snippet_truncated_at_4_kib() {
        let idx = Index::new();
        // Build a theorem body whose total exceeds 4 KiB but stays under
        // 12 lines so the *byte* limit is the binding constraint.
        let big_line = "x".repeat(500); // 500 bytes
        let mut body = String::from("\\begin{theorem}\n");
        for _ in 0..10 {
            // 10 * 500 = 5000 bytes > 4096.
            body.push_str(&big_line);
            body.push('\n');
        }
        body.push_str("\\label{thm:big}\n\\end{theorem}");
        extract_labels(&body, Path::new("a.tex"), &idx);
        let entry = idx.labels.get("thm:big").expect("label found");
        let snippet = &entry.snippet;
        // Truncation marker present.
        assert!(snippet.contains("% (truncated)"), "snippet = {snippet:?}");
        // ≤ 4 KiB + the marker line.
        assert!(
            snippet.len() <= SNIPPET_MAX_BYTES + TRUNCATED_MARKER.len() + 2,
            "snippet len = {}",
            snippet.len()
        );
        // ≤ 12 lines.
        let line_count = snippet.lines().count();
        assert!(
            line_count <= 13,
            "expected ≤ 13 lines, got {line_count}"
        );
    }
}
