//! Workspace file walker — port of `server/src/preamble.ts`.
//!
//! Walks the project root recursively, skipping known noise directories and
//! dot-directories, returning every `*.tex` and `*.bib` file's absolute
//! path.  Depth is bounded at 20 to mirror the Node implementation.
//!
//! Also exposes the URI / path normalisation helpers used by `main.rs`
//! (the Node side sends `file:///...` URIs over the wire).

use std::path::{Path, PathBuf};

/// Directories that are never worth scanning.
pub const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".texghost",
    "out",
    "dist",
    "target",
    "__pycache__",
    ".vscode",
    ".idea",
];

const MAX_DEPTH: usize = 20;

/// Recursively collect every `*.tex` and `*.bib` file under `root`.
/// Errors reading a directory are swallowed (matches Node behaviour).
pub fn collect_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            if SKIP_DIRS.iter().any(|s| *s == name.as_ref()) {
                continue;
            }
            // Skip dot-directories (`.git`, `.vscode`, …).
            if name.starts_with('.') && name.len() > 1 {
                continue;
            }
            walk(&path, depth + 1, out);
        } else if ft.is_file() {
            if name.ends_with(".tex") || name.ends_with(".bib") {
                out.push(path);
            }
        }
    }
}

/// Does `dir` contain any `*.tex` file directly inside it?
pub fn has_tex_files(dir: &Path) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        if let Ok(ft) = entry.file_type() {
            if ft.is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".tex") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Walk upward from `file_path`'s directory until the parent no longer
/// contains `.tex` files.  Returns that directory, or `None`.
pub fn infer_project_root(file_path: &Path) -> Option<PathBuf> {
    let mut dir = file_path.parent()?.to_path_buf();
    if !has_tex_files(&dir) {
        return None;
    }
    loop {
        let parent = dir.parent()?.to_path_buf();
        if parent == dir {
            break;
        }
        if !has_tex_files(&parent) {
            break;
        }
        dir = parent;
    }
    Some(dir)
}

/// Convert a `file://` URI or plain path into a normalised absolute path.
/// Returns `None` if the input cannot be parsed.
pub fn normalise_uri(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }
    let p = if let Some(rest) = raw.strip_prefix("file://") {
        let decoded = percent_decode(rest);
        // Windows: file:///C:/path → /C:/path → C:/path
        if cfg!(windows) && decoded.starts_with('/') {
            let bytes = decoded.as_bytes();
            if bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'/' || bytes[2] == b'\\') {
                decoded[1..].to_string()
            } else {
                decoded
            }
        } else {
            decoded
        }
    } else {
        raw.to_string()
    };
    Some(PathBuf::from(p))
}

/// Minimal percent-decode for path URIs (`%20` → space).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Stringify a path for JSON output — always absolute, always forward slashes.
pub fn json_path(p: &Path) -> String {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    abs.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_basic() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a%2Bb"), "a+b");
    }

    #[test]
    fn collect_files_finds_tex_and_bib() {
        let tmp = tempdir();
        std::fs::write(tmp.join("a.tex"), "% tex").unwrap();
        std::fs::write(tmp.join("b.bib"), "@article{x, year=1}").unwrap();
        std::fs::create_dir(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("sub/c.tex"), "% tex").unwrap();
        let files = collect_files(&tmp);
        assert_eq!(files.len(), 3);
        cleanup(&tmp);
    }

    #[test]
    fn skips_noise_dirs() {
        let tmp = tempdir();
        std::fs::create_dir(tmp.join("node_modules")).unwrap();
        std::fs::write(tmp.join("node_modules/x.tex"), "% tex").unwrap();
        std::fs::write(tmp.join("keep.tex"), "% tex").unwrap();
        let files = collect_files(&tmp);
        assert_eq!(files.len(), 1);
        cleanup(&tmp);
    }

    // ── tiny test-dir helpers ──────────────────────────────────────────

    fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let nonce: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        p.push(format!("latex-index-test-{}", nonce));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }
}
