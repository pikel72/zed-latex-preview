use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use base64::Engine;
use serde_json::{json, Value};

struct LspChild {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl LspChild {
    fn spawn() -> Self {
        let exe = env!("CARGO_BIN_EXE_latex-preview-lsp");
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn latex-preview-lsp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, msg: Value) {
        let body = serde_json::to_string(&msg).expect("json");
        write!(
            self.stdin,
            "Content-Length: {}\r\n\r\n{}",
            body.as_bytes().len(),
            body
        )
        .expect("write lsp");
        self.stdin.flush().expect("flush lsp");
    }

    fn recv(&mut self) -> Value {
        let mut content_len = None;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("read header");
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                content_len = Some(rest.trim().parse::<usize>().expect("content length"));
            }
        }
        let len = content_len.expect("Content-Length header");
        let mut body = vec![0u8; len];
        self.stdout.read_exact(&mut body).expect("read body");
        serde_json::from_slice(&body).expect("json response")
    }
}

impl Drop for LspChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn initialize(lsp: &mut LspChild) {
    initialize_with_root(lsp, Value::Null);
}

fn initialize_with_root(lsp: &mut LspChild, root_uri: Value) {
    let root_uri_field = if root_uri.is_null() {
        Value::Null
    } else {
        root_uri
    };
    let root_path_field = Value::Null;
    let params = json!({
        "processId": null,
        "rootUri": root_uri_field,
        "rootPath": root_path_field,
        "capabilities": {},
        "initializationOptions": {
            "scale": 1.0,
            "timeoutMs": 5000
        }
    });
    lsp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": params
    }));
    let init = lsp.recv();
    assert_eq!(init["id"], 1);
    assert!(init["result"]["capabilities"]["hoverProvider"]
        .as_bool()
        .unwrap());

    lsp.send(json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    }));
}

fn open_doc(lsp: &mut LspChild, uri: &str, text: &str) {
    lsp.send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "latex",
                "version": 1,
                "text": text
            }
        }
    }));
}

fn hover_value(
    lsp: &mut LspChild,
    id: u64,
    uri: &str,
    line: u64,
    character: u64,
) -> Option<String> {
    lsp.send(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    let hover = lsp.recv();
    assert_eq!(hover["id"], id);
    hover["result"]["contents"]["value"]
        .as_str()
        .map(str::to_string)
}

fn shutdown(lsp: &mut LspChild, id: u64) {
    lsp.send(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "shutdown",
        "params": null
    }));
    let shutdown = lsp.recv();
    assert_eq!(shutdown["id"], id);
}

#[test]
fn lsp_returns_math_hover_over_open_document() {
    let mut lsp = LspChild::spawn();
    initialize(&mut lsp);

    open_doc(
        &mut lsp,
        "file:///tmp/main.tex",
        "Inline $E = mc^2$ formula.",
    );
    let value = hover_value(&mut lsp, 2, "file:///tmp/main.tex", 0, 10).unwrap();
    assert!(
        value.starts_with("![formula](data:image/svg+xml;base64,"),
        "unexpected hover: {value}"
    );

    shutdown(&mut lsp, 3);
}

#[test]
fn lsp_returns_cite_ref_and_doc_hovers() {
    let mut lsp = LspChild::spawn();
    initialize(&mut lsp);

    open_doc(
        &mut lsp,
        "file:///tmp/refs.bib",
        "@article{einstein1905,\n  author = {Albert Einstein},\n  title = {Zur Elektrodynamik bewegter Korper},\n  year = {1905}\n}\n",
    );
    open_doc(
        &mut lsp,
        "file:///tmp/main.tex",
        "\\section{Intro}\\label{sec:intro}\nSee \\cite{einstein1905}, \\ref{sec:intro}, and \\usepackage{amsmath}.\n",
    );

    let cite = hover_value(&mut lsp, 2, "file:///tmp/main.tex", 1, 12).unwrap();
    assert!(
        cite.contains("Albert Einstein 1905"),
        "unexpected cite hover: {cite}"
    );
    assert!(
        cite.contains("Zur Elektrodynamik"),
        "unexpected cite hover: {cite}"
    );

    let reference = hover_value(&mut lsp, 3, "file:///tmp/main.tex", 1, 33).unwrap();
    assert!(
        reference.contains("Section: Intro"),
        "unexpected ref hover: {reference}"
    );

    let doc = hover_value(&mut lsp, 4, "file:///tmp/main.tex", 1, 61).unwrap();
    assert!(
        doc.contains("**amsmath** (package)"),
        "unexpected doc hover: {doc}"
    );

    shutdown(&mut lsp, 5);
}

#[test]
fn lsp_sanitizes_svg_for_formula_with_less_than_and_expanded_macro() {
    let mut lsp = LspChild::spawn();
    initialize(&mut lsp);

    let text = "\\def\\O{\\Omega}\nDefine space $C^{0,\\gamma}(\\O)$ by $$\nC^{0,\\gamma}(\\O):=\\{f\\in C(\\O):\\|f\\|_{C^{0,\\gamma}(\\O)}<+\\infty \\},\n$$\n";
    open_doc(&mut lsp, "file:///tmp/functional_1.tex", text);
    let value = hover_value(&mut lsp, 2, "file:///tmp/functional_1.tex", 2, 5).unwrap();
    assert!(
        value.starts_with("![formula](data:image/svg+xml;base64,"),
        "unexpected hover: {value}"
    );
    let encoded = value
        .split("base64,")
        .nth(1)
        .and_then(|s| s.strip_suffix(')'))
        .expect("data uri payload");
    let svg = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("base64 svg"),
    )
    .expect("utf8 svg");
    assert!(!svg.contains("data-latex"), "unsanitized SVG: {svg}");
    assert!(!svg.contains("<+\\infty"), "raw TeX leaked into SVG: {svg}");
    assert!(
        svg.contains("data-c=\"3A9\"") || svg.contains("1D6FA"),
        "macro did not expand to Omega-ish SVG: {svg}"
    );

    shutdown(&mut lsp, 3);
}

#[test]
fn lsp_exits_cleanly_after_shutdown_and_exit() {
    // Regression for the writer-channel deadlock: a proper
    // shutdown + exit + stdin EOF sequence must let the process exit on
    // its own within a few seconds.  We avoid `LspChild` here because its
    // `Drop` kills the child (masking exactly the bug we're guarding
    // against); we hold the `Child` directly so `try_wait` is observable.
    let exe = env!("CARGO_BIN_EXE_latex-preview-lsp");
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn latex-preview-lsp");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));

    let send = |stdin: &mut ChildStdin, msg: Value| {
        let body = serde_json::to_string(&msg).expect("json");
        write!(
            stdin,
            "Content-Length: {}\r\n\r\n{}",
            body.as_bytes().len(),
            body
        )
        .expect("write lsp");
        stdin.flush().expect("flush lsp");
    };
    let recv = |stdout: &mut BufReader<ChildStdout>| -> Value {
        let mut content_len = None;
        loop {
            let mut line = String::new();
            stdout.read_line(&mut line).expect("read header");
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
                content_len = Some(rest.trim().parse::<usize>().expect("content length"));
            }
        }
        let len = content_len.expect("Content-Length header");
        let mut body = vec![0u8; len];
        stdout.read_exact(&mut body).expect("read body");
        serde_json::from_slice(&body).expect("json response")
    };

    send(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": null,
                "capabilities": {},
                "initializationOptions": { "scale": 1.0, "timeoutMs": 5000 }
            }
        }),
    );
    let init = recv(&mut stdout);
    assert_eq!(init["id"], 1);
    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    );

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
    );
    let resp = recv(&mut stdout);
    assert_eq!(resp["id"], 99);

    send(
        &mut stdin,
        json!({"jsonrpc":"2.0","method":"exit","params":null}),
    );
    drop(stdin); // close pipe → reader EOF

    // 3 s is generous: the smoke test reliably exits in <50 ms.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("lsp did not exit within 3s after shutdown+exit+stdin EOF");
            }
        }
    };
    assert!(status.success(), "unexpected exit status: {status:?}");
}
