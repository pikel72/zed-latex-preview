# Plan - Rust-Primary LSP Refactor

> Status: **runtime cutover complete; legacy cleanup pending**
>
> Phases 0–5 are done: `latex-preview-lsp` is the Rust native LSP, Zed
> launches it through `src/lib.rs`, and math/cite/ref/doc hover all run
> entirely in Rust with `mathjax-svg-rs` rendering.  Phase 6 (CI cutover)
> and Phase 7 (delete legacy `server/` tree, remove old sidecar binary)
> remain.  See *Cutover status* below for the per-phase exit-criteria check.
>
> Scope: replace the current Node-primary LSP plus Rust sidecar split with a
> Rust-native language server as the main runtime. TypeScript/Node should leave
> the normal execution path. MathJax may remain as the renderer, but only behind
> a Rust API in the Rust LSP process.

---

## 1. Decision

The current architecture is not Rust-primary.

Today the Zed extension's WASM entry point is Rust, but the real language
server is `server/out/src/server.js`. That Node process owns LSP transport,
document sync, hover dispatch, MathJax rendering, cache policy, and fallback
behavior. The Rust binary under `latex-index/` is a best-effort sidecar for
workspace indexing and lookup.

The target architecture is:

```text
Zed
  -> Rust WASM extension stub
  -> Rust native LSP binary: latex-preview-lsp
       - LSP stdio transport
       - document cache and text sync
       - hover dispatch
       - workspace index, watcher, cite/ref/doc lookup
       - macro extraction and expansion
       - math-region detection
       - SVG renderer backend
```

Node is no longer the language server. It is not a required runtime for users.

---

## 2. Current Runtime Boundary

| Area | Current owner | Rust-primary target |
|------|---------------|---------------------|
| Zed extension registration | `src/lib.rs` | Keep as Rust WASM stub |
| LSP process | `server/src/server.ts` | Rust native binary |
| Document sync | `vscode-languageserver` `TextDocuments` | Rust document store |
| Hover dispatch | `server/src/hover.ts` | Rust hover module |
| Math scanner | `server/src/scanner.ts`, partial Rust port in `cursor.rs` | Rust only |
| Macro extraction | `server/src/macros.ts`, Rust workspace parser | Rust only |
| Workspace macro collection | `server/src/preamble.ts` + sidecar IPC | Rust index directly |
| Citation/ref/doc lookup | Rust sidecar via `rust_sidecar.ts` | Rust direct calls |
| Math SVG rendering | `server/src/render.ts` with `mathjax-full` | Rust renderer trait |
| Cache/data URI formatting | TypeScript | Rust |
| Tests | Node tests plus Rust sidecar tests | Rust unit/integration tests |

The existing `latex-index` crate already contains much of the Rust core:

- `cursor.rs`: cursor context and math-region detection.
- `labels.rs`: label extraction and snippet building.
- `bibtex.rs`: BibTeX parsing.
- `macros.rs`: macro extraction.
- `dict.rs`: package/command dictionary.
- `watcher.rs`: file watcher.
- `workspace.rs`: path and workspace walking helpers.
- `index.rs`: shared index state.

The refactor should promote this crate from sidecar to language server.

---

## 3. Renderer Feasibility

The only real TypeScript hard dependency is MathJax SVG rendering. The
repository currently uses `mathjax-full` from Node in `server/src/render.ts`.

This does not require a Node-primary architecture. A Rust renderer backend can
embed MathJax through a JavaScript engine. The initial candidate is
`mathjax-svg-rs`, which wraps MathJax in Rust through Boa. A local feasibility
check on Windows passed the crate's test suite:

```text
running 4 tests
test_render_tex_with_invalid_font_size ... ok
test_mathjax_render_tex ... ok
test_render_tex ... ok
test_mathjax_can_render_from_multiple_threads ... ok
```

Renderer backend options:

| Option | Fit | Notes |
|--------|-----|-------|
| `mathjax-svg-rs` | Best first candidate | No Node/Chrome runtime; uses Boa; local smoke tests pass |
| `mathjax_svg` | Possible fallback | Uses V8, heavier build/runtime footprint |
| `mathjax` crate | Poor fit | Tends toward Node/browser backends |
| `lo_math` | Not equivalent | Emits MathML/ODF, not current MathJax SVG behavior |
| Hand-written TeX renderer | Reject | Too large and likely worse compatibility |

The renderer must be hidden behind a trait so the rest of the LSP does not care
which backend is used:

```rust
trait Renderer {
    fn render(&self, request: RenderRequest) -> RenderResult;
}

struct RenderRequest {
    source: String,
    display: bool,
    scale: f64,
    color: ColorMode,
    timeout_ms: u64,
}
```

Required behavior parity with `server/src/render.ts`:

- Inline and display mode.
- User scale mapped to SVG size.
- `auto`, `black`, and `white` color handling.
- Timeout handling.
- Parse-error detection for invalid TeX and unknown commands.
- SVG extraction and sanitation.
- Data URI generation for markdown image hovers.
- Fallback to fenced TeX source when rendering fails.

---

## 4. Target Crate Layout

Prefer a small workspace split instead of one growing binary crate:

```text
crates/
  latex-preview-core/
    cursor.rs
    labels.rs
    bibtex.rs
    macros.rs
    dict.rs
    index.rs
    workspace.rs
    hover/
    render/
  latex-preview-lsp/
    main.rs
    protocol.rs
    documents.rs
    server.rs
src/
  lib.rs                  # Zed WASM extension stub
```

An incremental variant is acceptable if churn must stay smaller:

```text
latex-index/
  src/
    main.rs               # becomes LSP binary entry point
    sidecar_rpc.rs         # temporary legacy RPC code
    lsp.rs
    hover.rs
    render.rs
```

The important boundary is not the folder name. The important boundary is that
the Rust binary owns the LSP protocol and calls the indexing/rendering code
directly.

---

## 5. Implementation Phases

### Phase 0 - Baseline and Fixtures

Goal: freeze the current behavior before changing runtime ownership.

- Capture representative hover fixtures:
  - inline math.
  - display math.
  - macro expansion from current document.
  - macro expansion from sibling/preamble file.
  - cite hover.
  - ref hover for math env.
  - ref hover for theorem/section-like env.
  - doc hover for package and command.
  - invalid TeX fallback.
- Record expected markdown shapes, not byte-identical SVG unless needed.
- Keep current dirty worktree changes untouched unless they are part of the
  active task.

Exit criteria:

- Fixture set exists.
- Current Node LSP behavior is documented enough to compare against.

### Phase 1 - Rust LSP Skeleton

Goal: create a Rust binary that Zed can run as a language server.

- Add `latex-preview-lsp` binary.
- Choose `lsp-server` for a small synchronous stdio server, or `tower-lsp` if
  async lifecycle management becomes useful.
- Implement:
  - `initialize`
  - `shutdown`
  - `textDocument/didOpen`
  - `textDocument/didChange`
  - `textDocument/didClose`
  - `textDocument/hover`
- Return null hover initially.
- Add an integration test that speaks LSP over stdio.

Exit criteria:

- `cargo test` covers basic LSP initialize/open/hover/shutdown.
- The binary can be launched manually through `lsp.latex-preview.binary.path`.

### Phase 2 - Direct Rust Index Integration

Goal: remove sidecar IPC from the new path.

- Reuse existing index state directly inside the LSP server.
- Replace RPC methods with internal calls:
  - `update_file`
  - `close_file`
  - `cursor_context`
  - `lookup`
  - `doc_lookup`
  - `workspace_macros`
- Keep the watcher lifecycle owned by the Rust LSP state.
- Audit shutdown so watcher threads do not block process exit.

Exit criteria:

- Cite/ref/doc lookup works in Rust integration tests.
- No Node child process is needed for these features.

### Phase 3 - Hover Formatter Migration

Goal: port the TypeScript hover pipeline into Rust.

- Port `hoverFor` dispatch order:
  - cite
  - ref
  - doc
  - math
- Port cite markdown formatting from `server/src/cite_hover.ts`.
- Port ref markdown formatting from `server/src/ref_hover.ts`.
- Port doc markdown formatting from `server/src/doc_hover.ts`.
- Port fenced-code-block escaping.
- Port relative path shortening.
- Implement range conversion from byte offsets to LSP positions.

Exit criteria:

- Rust tests cover markdown shape for cite/ref/doc hovers.
- `textDocument/hover` returns Zed-compatible markdown hover payloads.

### Phase 4 - Math Hover Without Node

Goal: make math hover work entirely inside Rust.

- Finalize math-region detection in Rust.
- Finalize macro expansion in Rust, including current-document precedence.
- Add render cache keyed by:
  - expanded source.
  - macro set or macro version.
  - color.
  - scale.
  - display mode.
- Integrate `mathjax-svg-rs` behind the renderer trait.
- Reproduce `render.ts` behavior for:
  - color injection.
  - size conversion and padding.
  - parse error fallback.
  - timeout.
  - data URI encoding.

Exit criteria:

- Rust math hover fixtures pass.
- Invalid TeX returns fenced source instead of crashing.
- No Node process is used for math hover.

### Phase 5 - Zed Extension Launch Path

Goal: make the Rust LSP the normal extension runtime.

- Update `src/lib.rs` launch resolution:
  1. user-provided `lsp.latex-preview.binary.path`;
  2. bundled `latex-preview-lsp` binary;
  3. optional development fallback only if explicitly enabled.
- Remove the implicit Node fallback from the normal path.
- Keep initialization options forwarding.
- Update errors so missing Rust binary is explicit and actionable.

Exit criteria:

- Zed starts the Rust LSP through the extension stub.
- README no longer says Node.js 18+ is required for normal use.

### Phase 6 - Test and CI Cutover

Goal: make Rust tests the authoritative validation gate.

- Convert TS tests to Rust equivalents:
  - scanner.
  - macros.
  - render smoke.
  - hover.
  - cite hover.
  - ref hover.
  - doc hover.
  - config.
  - sidecar tests become LSP integration tests.
- Add release build smoke for `latex-preview-lsp`.
- Keep temporary Node tests only while comparing behavior.

Exit criteria:

- `cargo test` is the primary gate.
- Node test suite is no longer required to validate runtime behavior.

### Phase 7 - Remove Legacy Node Path

Goal: finish the Rust-primary cutover.

- Delete or archive `server/` runtime code after parity is verified.
- Remove `package.json` runtime dependency from user-facing instructions.
- Remove sidecar-specific settings:
  - `enabledSidecar`
  - `sidecarPath`
- Replace them with Rust LSP settings only if still needed.
- Update docs and diagrams.

Exit criteria:

- Normal extension install and run path contains no Node dependency.
- TypeScript is absent or limited to non-runtime tooling explicitly marked as
  legacy/development-only.

---

## 6. Validation Matrix

| Area | Required checks |
|------|-----------------|
| LSP lifecycle | initialize, open, change, close, hover, shutdown |
| Math scanning | `$...$`, `$$...$$`, `\(...\)`, `\[...\]`, equation/align/gather/multline |
| Comments/verbatim | skip math delimiters in comments and verbatim-like envs |
| Macro extraction | `\newcommand`, starred forms, `\renewcommand`, `\providecommand`, `\def`, `\DeclareMathOperator` |
| Macro expansion | zero-arg and multi-arg macros; current document overrides workspace |
| Rendering | inline/display, color, scale, timeout, invalid TeX fallback |
| Cite hover | missing key, full BibTeX entry, abstract, path formatting |
| Ref hover | math env SVG, theorem/section fenced text, missing label fallthrough |
| Doc hover | package, command, unknown term fallthrough |
| Workspace | sibling preamble files, `.bib` files, watcher updates, ignored directories |
| Packaging | extension launches bundled binary on Windows/macOS/Linux |

---

## 7. Risks

### Renderer compatibility

`mathjax-svg-rs` proves that in-process Rust rendering is feasible, but it must
be tested against this extension's expected output. The main risks are package
coverage, error detection, sizing, and output differences from `mathjax-full`
as used today.

Mitigation: keep the renderer behind a trait and test markdown behavior first,
with focused SVG assertions only for properties that matter to Zed display.

### Binary packaging

Zed extensions can return a native language server command, but shipping
platform-specific binaries needs an explicit release layout.

Mitigation: keep `binary.path` override for development, document the bundled
binary lookup, and add release smoke tests per target.

### Watcher lifecycle

The current sidecar owns a watcher thread. As a long-lived LSP server, shutdown
and reinitialization behavior matter more.

Mitigation: make watcher ownership explicit in LSP state and test shutdown.

### Behavior drift during port

The TypeScript hover path has many small formatting decisions.

Mitigation: port tests before deleting TypeScript and compare markdown shape
for representative fixtures.

---

## 8. Non-Goals

- Rewriting MathJax itself.
- Implementing completions, diagnostics, build tasks, or texlab replacement.
- Making the Zed WASM extension host run the LSP logic directly.
- Preserving Node fallback indefinitely.
- Pursuing byte-for-byte SVG equality unless Zed rendering requires it.

---

## 9. Definition of Done

The refactor is complete when:

- `latex-preview-lsp` is a Rust native binary and owns LSP transport.
- Zed launches that binary from `src/lib.rs`.
- Math, cite, ref, and doc hover work without starting Node.
- `cargo test` covers the runtime behavior.
- User-facing docs no longer require Node.js.
- The old Node LSP path is removed or clearly marked legacy and unused.

---

## 10. Cutover Status (as of 2026-06-17)

| Phase | Status | Notes |
|-------|--------|-------|
| 0 — Baseline fixtures | ✅ done | Behaviour fixtures captured in `latex-index/tests/lsp_integration.rs` (math, cite/ref/doc, SVG sanitisation) |
| 1 — Rust LSP skeleton | ✅ done | `lsp_main.rs` uses `lsp-server`; initialize/hover/didOpen/didChange/didClose/shutdown wired |
| 2 — Direct index integration | ✅ done | No IPC on the hover path; watcher owned by `LspServer`; `WatcherHandle::Drop` joins the thread |
| 3 — Hover formatter migration | ✅ done | cite/ref/doc dispatch + markdown shape ported; `byte_range_to_lsp_range` converts in UTF-16 |
| 4 — Math hover without Node | ✅ done | macro extraction + expansion in Rust; `mathjax-svg-rs` behind a single dedicated render worker; LRU cache (cap 64); error fallback uses *expanded* source |
| 5 — Zed extension launch path | ✅ done | `src/lib.rs` resolves user-path → PATH → `bin/` → `target/release` → `target/debug`; Node fallback removed; missing-binary error is explicit |
| 6 — Test and CI cutover | ⏳ partial | Rust tests are the primary gate (49 unit + 3 LSP-integration, all green). CI workflow still builds the old `latex-index` sidecar and runs `npm test`; needs to switch to `latex-preview-lsp` build + release-binary artifact per OS |
| 7 — Remove legacy Node path | ⏳ pending | `server/` tree still present and marked legacy via `server/README.md`; `latex-index/src/main.rs` sidecar binary still built; `enabledSidecar` / `sidecarPath` config keys still in the TS server. Delete after Phase 6 is green on CI |

### Known follow-ups outside Phases 6–7

- **Scanner deduplication.** `tokenize_math` and friends are defined twice
  (cursor.rs and lsp_main.rs).  Plan §4 calls for `latex-preview-core` to
  hold them once.  Functionally fine, but should be unified before adding
  more scanner work.
- **Per-hover work.** `find_math_region` re-tokenises the whole document
  even when `cursor_context` already located it.  Tolerable today thanks
  to the render cache; address as part of the scanner unification above.
- **Render cancellation.** A render that overshoots `timeoutMs` still
  finishes on the worker (single-threaded, so it only blocks the *next*
  render, not the LSP loop).  True cancellation needs upstream support
  in `mathjax-svg-rs` / Boa.
