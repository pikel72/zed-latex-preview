//! End-to-end IPC integration tests for the `latex-index` Rust sidecar.
//!
//! Each test spawns a fresh subprocess and drives it over stdio using the
//! NDJSON JSON-RPC 2.0 protocol.  See `src/main.rs` and `src/lsp_codec.rs`
//! for the protocol and dispatcher.
//!
//! These tests are `#[ignore]`-d by default because they hang under
//! `cargo test`'s output-capture mode on Windows (process-spawning
//! interacts poorly with libtest's pipe capture).  They pass reliably
//! when run directly:
//!
//! ```bash
//! cargo test -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The binary path is taken from the `LATEX_INDEX_PATH` environment
//! variable; if unset it falls back to the conventional release build at
//! `target/release/latex-index(.exe)`.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::{json, Value};

// ── harness ────────────────────────────────────────────────────────────

const PROTOCOL_VERSION: u32 = 1;

fn binary_path() -> PathBuf {
    if let Ok(p) = std::env::var("LATEX_INDEX_PATH") {
        return PathBuf::from(p);
    }
    // Walk up from CARGO_MANIFEST_DIR (latex-index/) to the crate root and
    // use the conventional release-build path.  Both `.exe` and bare names
    // are tried so this works on Windows and Unix.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut p = PathBuf::from(manifest_dir);
    p.push("target");
    p.push("release");
    let exe_name = if cfg!(windows) {
        "latex-index.exe"
    } else {
        "latex-index"
    };
    p.push(exe_name);
    p
}

/// Shared stdio state for one spawned sidecar.
struct Sidecar {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: AtomicI64,
}

impl Sidecar {
    fn spawn() -> Self {
        let bin = binary_path();
        let mut cmd = Command::new(&bin);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn {:?}: {e}", bin));
        let stdin = child
            .stdin
            .take()
            .expect("sidecar stdin must be piped");
        let stdout = child
            .stdout
            .take()
            .expect("sidecar stdout must be piped");
        Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            next_id: AtomicI64::new(1),
        }
    }

    fn next_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send one JSON-RPC request and return the matching response.
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_string(&req)
            .unwrap_or_else(|e| panic!("serialise request: {e}"));
        self.stdin
            .write_all(line.as_bytes())
            .expect("write to sidecar stdin");
        self.stdin.write_all(b"\n").expect("write newline");
        self.stdin.flush().expect("flush sidecar stdin");

        // Drain stdout lines until we find the matching id.  Any earlier
        // lines (e.g. responses to notifications, if we ever sent any) are
        // silently skipped; we always want exactly one response per call.
        loop {
            let mut buf = String::new();
            let read = self
                .stdout
                .read_line(&mut buf)
                .expect("read from sidecar stdout");
            assert!(read > 0, "sidecar closed stdout before responding to id={id}");
            let trimmed = buf.trim();
            assert!(!trimmed.is_empty(), "sidecar returned empty line");
            let v: Value = serde_json::from_str(trimmed)
                .unwrap_or_else(|e| panic!("invalid JSON from sidecar: {e}; line={trimmed:?}"));
            // Skip unsolicited notifications (no id); we don't emit any
            // in these tests but stay defensive.
            if v.get("id").is_none_or(Value::is_null) {
                continue;
            }
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    fn shutdown(&mut self) {
        // Drop the buffered writer to flush and close the pipe; the
        // sidecar's blocking read_line then returns EOF and it exits.
        self.stdin
            .flush()
            .expect("flush sidecar stdin before shutdown");
        // Take the underlying pipe out of `self.child` so dropping the
        // writer doesn't double-close it (and so Drop doesn't see the
        // child without a stdin handle).
        if let Some(stdin) = self.child.stdin.take() {
            drop(stdin);
        }
        let _ = self.child.wait();
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        // Best-effort cleanup if the test forgot to call `shutdown`.
        // Drop the writer first so its destructor doesn't try to flush
        // into a child we've already killed.
        if let Some(stdin) = self.child.stdin.take() {
            drop(stdin);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn initialize(s: &mut Sidecar) {
    let resp = s.rpc(
        "initialize",
        json!({ "rootUri": null, "version": PROTOCOL_VERSION }),
    );
    let result = resp
        .get("result")
        .unwrap_or_else(|| panic!("initialize: no result; resp={resp}"));
    assert_eq!(
        result.get("ok").and_then(Value::as_bool),
        Some(true),
        "initialize: ok should be true; got {result}"
    );
    let kinds = result
        .get("capabilities")
        .and_then(|c| c.get("kinds"))
        .and_then(Value::as_array)
        .expect("initialize: capabilities.kinds must be an array");
    for expected in ["cite", "ref", "math"] {
        assert!(
            kinds.iter().any(|k| k.as_str() == Some(expected)),
            "initialize: capabilities.kinds missing {expected}; got {kinds:?}"
        );
    }
}

// ── 1. handshake ───────────────────────────────────────────────────────

#[test]
#[ignore = "passes when run with --nocapture"]
fn handshake_initialize_then_ping() {
    let mut s = Sidecar::spawn();
    initialize(&mut s);

    let pong = s.rpc("ping", json!({}));
    let result = pong
        .get("result")
        .unwrap_or_else(|| panic!("ping: no result; resp={pong}"));
    assert_eq!(
        result.get("ok").and_then(Value::as_bool),
        Some(true),
        "ping: result.ok should be true; got {result}"
    );
    let uptime = result
        .get("uptime_ms")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("ping: result.uptime_ms missing/non-int; got {result}"));
    assert!(
        uptime < 60_000,
        "ping: uptime_ms {uptime} implausibly large for a fresh process"
    );

    // Dropping the sidecar triggers clean exit.
    s.shutdown();
}

// ── 2. label + cite flow ───────────────────────────────────────────────

#[test]
#[ignore = "passes when run with --nocapture"]
fn label_and_cite_round_trip() {
    let mut s = Sidecar::spawn();
    initialize(&mut s);

    let tex_uri = "file:///tmp/test.tex";
    let tex_text = "\\begin{equation}\n\\label{eq:foo}\nE = mc^2\n\\end{equation}\n\\cite{einstein1905}\n";
    let upd_tex = s.rpc(
        "update_file",
        json!({ "uri": tex_uri, "text": tex_text }),
    );
    let tex_result = upd_tex
        .get("result")
        .unwrap_or_else(|| panic!("update_file (.tex): no result; resp={upd_tex}"));
    assert_eq!(
        tex_result.get("ok").and_then(Value::as_bool),
        Some(true),
        "update_file (.tex): ok != true; got {tex_result}"
    );

    let bib_uri = "file:///tmp/refs.bib";
    let bib_text = "@article{einstein1905,\n  author = {Einstein},\n  title  = {On the electrodynamics of moving bodies},\n  year   = {1905}\n}\n";
    let upd_bib = s.rpc(
        "update_file",
        json!({ "uri": bib_uri, "text": bib_text }),
    );
    let bib_result = upd_bib
        .get("result")
        .unwrap_or_else(|| panic!("update_file (.bib): no result; resp={upd_bib}"));
    assert_eq!(
        bib_result.get("ok").and_then(Value::as_bool),
        Some(true),
        "update_file (.bib): ok != true; got {bib_result}"
    );

    // cursor_context at the byte offset of "einstein1905" inside \cite{...}.
    // \cite{einstein1905} starts after the equation block + a newline.
    // Be careful: \label{eq:foo} should *not* trigger a cite response.
    let cite_off = tex_text
        .find("\\cite{einstein1905}")
        .expect("cite in test text")
        + "\\cite{".len()
        + 1; // land somewhere inside the key, not on the brace
    let ctx = s.rpc(
        "cursor_context",
        json!({ "uri": tex_uri, "offset": cite_off }),
    );
    let ctx_result = ctx
        .get("result")
        .unwrap_or_else(|| panic!("cursor_context: no result; resp={ctx}"));
    assert_eq!(
        ctx_result.get("kind").and_then(Value::as_str),
        Some("cite"),
        "cursor_context: kind should be 'cite'; got {ctx_result}"
    );
    assert_eq!(
        ctx_result.get("key").and_then(Value::as_str),
        Some("einstein1905"),
        "cursor_context: key should be 'einstein1905'; got {ctx_result}"
    );

    let lookup = s.rpc(
        "lookup",
        json!({ "key": "einstein1905", "kind": "cite" }),
    );
    let lookup_result = lookup
        .get("result")
        .unwrap_or_else(|| panic!("lookup: no result; resp={lookup}"));
    assert_eq!(
        lookup_result.get("found").and_then(Value::as_bool),
        Some(true),
        "lookup(einstein1905,cite): found != true; got {lookup_result}"
    );
    let entry = lookup_result
        .get("entry")
        .unwrap_or_else(|| panic!("lookup: entry missing when found=true; got {lookup_result}"));
    assert_eq!(
        entry.get("entry_type").and_then(Value::as_str),
        Some("article"),
        "lookup entry_type should be 'article'; got {entry}"
    );
    assert_eq!(
        entry.get("key").and_then(Value::as_str),
        Some("einstein1905"),
        "lookup entry.key should be 'einstein1905'; got {entry}"
    );

    s.shutdown();
}

// ── 3. lookup miss ─────────────────────────────────────────────────────

#[test]
#[ignore = "passes when run with --nocapture"]
fn lookup_miss_for_unknown_key() {
    let mut s = Sidecar::spawn();
    initialize(&mut s);

    // No update_file — index is empty.
    let resp = s.rpc(
        "lookup",
        json!({ "key": "nonexistent_key", "kind": "cite" }),
    );
    let result = resp
        .get("result")
        .unwrap_or_else(|| panic!("lookup: no result; resp={resp}"));
    assert_eq!(
        result.get("found").and_then(Value::as_bool),
        Some(false),
        "lookup of unknown cite key should return found=false; got {result}"
    );
    assert!(
        result.get("entry").is_none(),
        "lookup miss should not include an entry; got {result}"
    );

    s.shutdown();
}

// ── 4. bad method ──────────────────────────────────────────────────────

#[test]
#[ignore = "passes when run with --nocapture"]
fn bad_method_returns_method_not_found() {
    let mut s = Sidecar::spawn();
    initialize(&mut s);

    let resp = s.rpc("no_such_method", json!({}));
    let error = resp
        .get("error")
        .unwrap_or_else(|| panic!("bad method should produce error envelope; got {resp}"));
    assert_eq!(
        error.get("code").and_then(Value::as_i64),
        Some(-32601),
        "bad method: error.code should be -32601 (METHOD_NOT_FOUND); got {error}"
    );
    let msg = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        msg.contains("no_such_method"),
        "bad method: error.message should mention the unknown method; got {msg:?}"
    );

    s.shutdown();
}

// ── 5. watcher picks up external changes ─────────────────────────────

/// Mirrors `initialize` but passes a real `rootUri` so the sidecar
/// spawns the file watcher.  Spec §7.3.
fn initialize_with_root(s: &mut Sidecar, root: &Path) {
    let root_uri = format!("file://{}", root.display());
    let resp = s.rpc(
        "initialize",
        json!({ "rootUri": root_uri, "version": PROTOCOL_VERSION }),
    );
    let result = resp
        .get("result")
        .unwrap_or_else(|| panic!("initialize_with_root: no result; resp={resp}"));
    assert_eq!(
        result.get("ok").and_then(Value::as_bool),
        Some(true),
        "initialize_with_root: ok should be true; got {result}"
    );
}

/// Tiny tempdir helper for the integration test (avoids the
/// `tempfile` crate; mirrors `workspace.rs:208`).
fn test_tempdir() -> PathBuf {
    let mut p = std::env::temp_dir();
    let nonce: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    p.push(format!("latex-index-itest-{}", nonce));
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

#[test]
#[ignore = "passes when run with --nocapture"]
fn external_change_picks_up_new_label() {
    let tmp = test_tempdir();
    let mut s = Sidecar::spawn();
    initialize_with_root(&mut s, &tmp);

    // Give the watcher a beat to install before we start writing.
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Write a .tex file that defines a label we can look up.
    let tex_path = tmp.join("paper.tex");
    let tex_text = "\\begin{equation}\n\\label{eq:watcher}\nE = mc^2\n\\end{equation}\n";
    std::fs::write(&tex_path, tex_text).expect("write paper.tex");

    // Wait for the debouncer (200ms) + a small buffer for the read.
    std::thread::sleep(std::time::Duration::from_millis(500));

    let lookup = s.rpc(
        "lookup",
        json!({ "key": "eq:watcher", "kind": "ref" }),
    );
    let result = lookup
        .get("result")
        .unwrap_or_else(|| panic!("lookup: no result; resp={lookup}"));
    assert_eq!(
        result.get("found").and_then(Value::as_bool),
        Some(true),
        "watcher should have picked up eq:watcher from disk; got {result}"
    );

    s.shutdown();
    let _ = std::fs::remove_dir_all(&tmp);
}
