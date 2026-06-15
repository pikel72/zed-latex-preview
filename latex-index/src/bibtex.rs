//! Hand-rolled BibTeX parser.
//!
//! See `docs/plan-ref-cite-hover.md` Section 5.2.  Scope: just enough to
//! produce a hover preview — author, title, year, journal/publisher, etc.
//!
//! Two passes:
//!   1.  Collect every `@string` macro into a `HashMap`.
//!   2.  Walk every `@type{key, field = value, …}` and substitute the
//!       string macros into the field values.
//!
//! Nested `{…}` is honoured; `"…"` is also accepted but does not nest;
//! concatenations of bare identifiers and `@string` macros inside braces are
//! collapsed into a single string.  Comments (`%…\n` outside entries and
//! `@comment{…}` / `@preamble{…}` / `@ignore{…}` blocks) are skipped.

use crate::index::{BibEntry, Index};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

// ── low-level tokeniser helpers ────────────────────────────────────────

fn starts_with_word(text: &[u8], at: usize, word: &[u8]) -> bool {
    at + word.len() <= text.len() && &text[at..at + word.len()] == word
}

fn skip_ws(text: &[u8], mut at: usize) -> usize {
    while at < text.len() && (text[at] == b' ' || text[at] == b'\t' || text[at] == b'\n' || text[at] == b'\r') {
        at += 1;
    }
    at
}

fn skip_to_eol(text: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < text.len() && text[i] != b'\n' {
        i += 1;
    }
    if i < text.len() { i + 1 } else { text.len() }
}

/// Read a balanced `{…}` starting at `at` (which must point at `{`).
/// Returns `(inner_start, inner_end, end)`.
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

/// Read a balanced `"…"` quoted string starting at `at` (which must point
/// at `"`).  No nesting inside.  Returns `(inner_start, inner_end, end)`.
fn read_quoted(text: &[u8], at: usize) -> Option<(usize, usize, usize)> {
    if text.get(at) != Some(&b'"') {
        return None;
    }
    let mut i = at + 1;
    while i < text.len() {
        match text[i] {
            b'\\' => {
                i = (i + 2).min(text.len());
                continue;
            }
            b'"' => {
                return Some((at + 1, i, i + 1));
            }
            _ => i += 1,
        }
    }
    None
}

/// Read an identifier `[A-Za-z0-9_:-]*` starting at `at`.  Returns the
/// slice `(start, end)` or `None`.
fn read_ident(text: &[u8], at: usize) -> Option<(usize, usize)> {
    let start = at;
    let mut i = at;
    while i < text.len() {
        let c = text[i];
        if c.is_ascii_alphanumeric() || c == b'_' || c == b':' || c == b'-' {
            i += 1;
        } else {
            break;
        }
    }
    if i == start {
        None
    } else {
        Some((start, i))
    }
}

// ── value parser ───────────────────────────────────────────────────────

/// Read a single field value (the part after `field =`).  Three forms:
///   * `{…}` (possibly multi-line, possibly nested)
///   * `"…"`
///   * Concatenation of bare identifiers / numbers / `@string` macros.
/// Concatenations are joined by a single space in the output.
fn read_value<'a>(
    text: &'a [u8],
    at: usize,
    strings: &HashMap<String, String>,
) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut i = skip_ws(text, at);
    let mut first = true;
    loop {
        if i >= text.len() {
            break;
        }
        match text[i] {
            b',' | b'}' | b')' => break,
            b'{' => {
                if let Some((a, b, end)) = read_braced(text, i) {
                    if !first {
                        out.push(' ');
                    }
                    let inner = std::str::from_utf8(&text[a..b]).unwrap_or("");
                    out.push_str(inner);
                    i = end;
                    first = false;
                } else {
                    return None;
                }
            }
            b'"' => {
                if let Some((a, b, end)) = read_quoted(text, i) {
                    if !first {
                        out.push(' ');
                    }
                    let inner = std::str::from_utf8(&text[a..b]).unwrap_or("");
                    out.push_str(inner);
                    i = end;
                    first = false;
                } else {
                    return None;
                }
            }
            b'#' => {
                // BibTeX concatenation operator — ignored.
                i += 1;
            }
            b'@' => {
                // @string reference.
                i += 1;
                let (s, e) = read_ident(text, i)?;
                let name = std::str::from_utf8(&text[s..e]).ok()?.to_string();
                if let Some(v) = strings.get(&name) {
                    if !first {
                        out.push(' ');
                    }
                    out.push_str(v);
                }
                i = e;
                first = false;
            }
            _ => {
                if let Some((s, e)) = read_ident(text, i) {
                    if !first {
                        out.push(' ');
                    }
                    let v = std::str::from_utf8(&text[s..e]).unwrap_or("");
                    out.push_str(v);
                    i = e;
                    first = false;
                } else {
                    // Unknown — skip one byte and keep going.
                    i += 1;
                }
            }
        }
        i = skip_ws(text, i);
    }
    Some((out.trim().to_string(), i))
}

// ── @string pass ───────────────────────────────────────────────────────

fn collect_strings(text: &[u8], strings: &mut HashMap<String, String>) {
    let mut i = 0usize;
    while i < text.len() {
        if text[i] == b'%' {
            i = skip_to_eol(text, i + 1);
            continue;
        }
        if text[i] == b'@' {
            // Try to read @string{ name = value }
            let after_at = i + 1;
            // Allow whitespace inside the `@string` tag.
            let tag_start = skip_ws(text, after_at);
            if starts_with_word(text, tag_start, b"string") {
                let mut probe = tag_start + b"string".len();
                probe = skip_ws(text, probe);
                if text.get(probe) == Some(&b'{') {
                    // Walk through the brace block, looking for `name = value` pairs.
                    let (_s, _e, end_of_block) = match read_braced(text, probe) {
                        Some(t) => t,
                        None => {
                            i = probe + 1;
                            continue;
                        }
                    };
                    // Parse the inside for `name = value , …`.
                    let mut p = probe + 1;
                    loop {
                        p = skip_ws(text, p);
                        if p >= end_of_block || text[p] == b'}' {
                            break;
                        }
                        if let Some((ns, ne)) = read_ident(text, p) {
                            p = skip_ws(text, ne);
                            if text.get(p) == Some(&b'=') {
                                p = skip_ws(text, p + 1);
                                let empty = HashMap::new();
                                if let Some((val, after)) = read_value(text, p, &empty) {
                                    let name = std::str::from_utf8(&text[ns..ne])
                                        .unwrap_or("")
                                        .to_string();
                                    strings.insert(name, val);
                                    p = after;
                                }
                            }
                        } else {
                            p += 1;
                        }
                    }
                    i = end_of_block;
                    continue;
                }
            }
            // Skip `@comment`, `@preamble`, `@ignore` blocks wholesale.
            for skip_tag in ["comment", "preamble", "ignore"] {
                let after_at = i + 1;
                let tag_start = skip_ws(text, after_at);
                if starts_with_word(text, tag_start, skip_tag.as_bytes()) {
                    let mut probe = tag_start + skip_tag.len();
                    probe = skip_ws(text, probe);
                    if text.get(probe) == Some(&b'{') {
                        if let Some((_, _, end)) = read_braced(text, probe) {
                            i = end;
                            break;
                        }
                    }
                    if text.get(probe) == Some(&b'(') {
                        if let Some(end) = find_matching_paren(text, probe) {
                            i = end;
                            break;
                        }
                    }
                }
            }
        }
        i += 1;
    }
}

fn find_matching_paren(text: &[u8], at: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut i = at + 1;
    while i < text.len() {
        match text[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' | b'(' => {
                depth += 1;
                i += 1;
            }
            b'}' | b')' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => i += 1,
        }
    }
    None
}

// ── main parser ────────────────────────────────────────────────────────

/// Parse a `.bib` file and insert every entry into `index`.  Existing
/// entries from `path` are dropped first.
pub fn parse_bibtex(text: &str, path: &Path, index: &Index) {
    let bytes = text.as_bytes();
    index.bib.retain(|_, v| v.file != path);

    let mut strings: HashMap<String, String> = HashMap::new();
    collect_strings(bytes, &mut strings);

    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            i = skip_to_eol(bytes, i + 1);
            continue;
        }
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        // Try to parse `@type{ key, field = value, … }`.
        let entry_start = i;
        let after_at = i + 1;
        let tag_start = skip_ws(bytes, after_at);
        if let Some((ty_s, ty_e)) = read_ident(bytes, tag_start) {
            let entry_type = std::str::from_utf8(&bytes[ty_s..ty_e])
                .unwrap_or("")
                .to_lowercase();
            // Skip the well-known no-ops.
            if matches!(
                entry_type.as_str(),
                "string" | "comment" | "preamble" | "ignore"
            ) {
                // @string is handled by collect_strings.  For the others
                // we still need to skip their body so we don't try to
                // re-parse them.
                if entry_type != "string" {
                    let mut probe = ty_e;
                    probe = skip_ws(bytes, probe);
                    if bytes.get(probe) == Some(&b'{') {
                        if let Some((_, _, end)) = read_braced(bytes, probe) {
                            i = end;
                            continue;
                        }
                    } else if bytes.get(probe) == Some(&b'(') {
                        if let Some(end) = find_matching_paren(bytes, probe) {
                            i = end;
                            continue;
                        }
                    }
                }
                i = ty_e;
                continue;
            }

            let mut probe = ty_e;
            probe = skip_ws(bytes, probe);
            if bytes.get(probe) != Some(&b'{') {
                i = ty_e;
                continue;
            }
            if let Some((_, body_end, block_end)) = read_braced(bytes, probe) {
                // Parse body: `key , field = value , …`
                let mut p = probe + 1;
                p = skip_ws(bytes, p);
                let key = match read_entry_key(bytes, p) {
                    Some((s, e, key)) => {
                        p = e;
                        key
                    }
                    None => {
                        i = block_end;
                        continue;
                    }
                };
                let _ = body_end;
                p = skip_ws(bytes, p);
                if bytes.get(p) == Some(&b',') {
                    p += 1;
                }

                let mut fields: BTreeMap<String, String> = BTreeMap::new();
                loop {
                    p = skip_ws(bytes, p);
                    if p >= block_end || bytes[p] == b'}' {
                        break;
                    }
                    if let Some((fs, fe)) = read_ident(bytes, p) {
                        let fname = std::str::from_utf8(&bytes[fs..fe])
                            .unwrap_or("")
                            .to_lowercase();
                        p = skip_ws(bytes, fe);
                        if bytes.get(p) == Some(&b'=') {
                            p = skip_ws(bytes, p + 1);
                            if let Some((val, after)) = read_value(bytes, p, &strings) {
                                if !fname.is_empty() {
                                    fields.insert(fname, val);
                                }
                                p = after;
                            } else {
                                break;
                            }
                        } else {
                            p = fe + 1;
                        }
                    } else {
                        p += 1;
                    }
                }

                let entry = BibEntry {
                    key,
                    file: path.to_path_buf(),
                    offset: entry_start,
                    fields,
                    entry_type,
                };
                index.bib.insert(entry.key.clone(), entry);
                i = block_end;
                continue;
            }
        }
        i += 1;
    }
}

/// Inside an entry body, read the citation key (everything up to the first
/// `,` or whitespace).
fn read_entry_key(text: &[u8], at: usize) -> Option<(usize, usize, String)> {
    let mut i = at;
    while i < text.len() {
        let c = text[i];
        if c == b',' || c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
            break;
        }
        i += 1;
    }
    let raw = std::str::from_utf8(&text[at..i]).ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some((at, i, raw.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_article() {
        let idx = Index::new();
        let bib = r#"
@article{einstein1905,
  author = {Einstein, A.},
  title  = {On the electrodynamics of moving bodies},
  year   = 1905,
  journal = {Annalen der Physik}
}
"#;
        parse_bibtex(bib, Path::new("r.bib"), &idx);
        let e = idx.bib.get("einstein1905").unwrap();
        assert_eq!(e.entry_type, "article");
        assert_eq!(e.fields.get("year").unwrap(), "1905");
        assert_eq!(e.fields.get("title").unwrap(), "On the electrodynamics of moving bodies");
    }

    #[test]
    fn parses_book() {
        let idx = Index::new();
        let bib = r#"@book{knuth1986, author = {Knuth, D.}, title = {The TeXbook}, publisher = {Addison-Wesley}, year = 1986}"#;
        parse_bibtex(bib, Path::new("r.bib"), &idx);
        let e = idx.bib.get("knuth1986").unwrap();
        assert_eq!(e.entry_type, "book");
        assert_eq!(e.fields.get("publisher").unwrap(), "Addison-Wesley");
    }

    #[test]
    fn nested_braces_in_field() {
        let idx = Index::new();
        let bib = r#"@misc{x, title = {Foo {Bar} Baz}}"#;
        parse_bibtex(bib, Path::new("r.bib"), &idx);
        let e = idx.bib.get("x").unwrap();
        assert_eq!(e.fields.get("title").unwrap(), "Foo {Bar} Baz");
    }

    #[test]
    fn string_substitution() {
        let idx = Index::new();
        let bib = r#"
@string{jp = "Journal of Physics"}
@article{a, journal = jp, year = 2020}
"#;
        parse_bibtex(bib, Path::new("r.bib"), &idx);
        let e = idx.bib.get("a").unwrap();
        assert_eq!(e.fields.get("journal").unwrap(), "Journal of Physics");
    }

    #[test]
    fn multiline_value() {
        let idx = Index::new();
        let bib = "@article{x,\n  abstract = {Line one.\n\nLine three.},\n  year = 2020\n}";
        parse_bibtex(bib, Path::new("r.bib"), &idx);
        let e = idx.bib.get("x").unwrap();
        assert!(e.fields.get("abstract").unwrap().contains("Line one."));
    }

    #[test]
    fn malformed_does_not_crash() {
        let idx = Index::new();
        let bib = "@article{broken, = value without name\n@article{ok, year=2020}";
        parse_bibtex(bib, Path::new("r.bib"), &idx);
        assert!(idx.bib.get("ok").is_some());
    }

    #[test]
    fn quoted_field_value() {
        let idx = Index::new();
        let bib = r#"@misc{x, title = "Quoted title"}"#;
        parse_bibtex(bib, Path::new("r.bib"), &idx);
        let e = idx.bib.get("x").unwrap();
        assert_eq!(e.fields.get("title").unwrap(), "Quoted title");
    }
}
