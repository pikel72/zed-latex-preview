# Phase 2 Design — ref-hover preview + file watcher

**Status:** design draft, awaiting approval
**Date:** 2026-06-16
**Scope:** the implementation work behind `docs/plan-ref-cite-hover.md` §9 "Phase 2"
**Out of scope:** Phase 1 close-out items (async `workspace_macros`, sidecar auto-restart, macos-arm64 CI runner) and Phase 3 items (xr, completionProvider).

---

## 1. Goals

A user can hover over `\ref{eq:foo}` (and the six other ref-family commands: `\eqref`, `\cref`, `\Cref`, `\autoref`, `\nameref`, `\pageref`) and see a rich markdown preview of the labelled site, including:

- the surrounding source as a code block (3–10 lines depending on the environment),
- a header line summarising what the label attaches to (`Theorem: <caption>`, `Equation`, `Section: <title>`),
- a `file:line` pointer so the user can jump.

A second, independent deliverable: when Zotero / Better BibTeX / `touch` writes a `.bib` or `.tex` file outside LSP, the sidecar picks the change up within ~250 ms and the next hover reflects the new state — without the user having to re-open the file in Zed.

A third, smaller deliverable: hovering over `\usepackage{<pkg>}` (or a bare command name like `\textbf`) shows a short description sourced from a bundled dictionary in the sidecar. Falls back to no-op when the command/package is not in the dictionary.

---

## 2. Architecture

Three largely independent pieces.

### 2.1 ref-hover

```
  server/src/hover.ts               latex-index                    server/src/ref_hover.ts
  ────────────────────              ─────────────                  ───────────────────────
  1. cursor_context(uri, off)  ───▶ cursor.rs
                                   detects \ref/\eqref/...
                                   returns {kind: "ref", key, range}
  2. lookup(key, "ref")        ───▶ labels::extract_labels
                                   returns LabelRef { ..., snippet }
  3. refHoverFor(lookup, range) ◀─── (TS side) formats markdown
       returns HoverResult
```

No new IPC method for ref-hover. The existing `cursor_context` and `lookup("ref")` cover it; we add a `snippet` field to the `LabelRef` payload so Node can render without re-reading the file.

The doc-hover piece (2.3) does add a new IPC method (`doc_lookup`); ref-hover is the "no new IPC" half of the work.

### 2.2 File watcher

```
  init
   │
   ▼
  walkdir(root)  ──── seed list of .tex / .bib paths
   │
   ▼
  notify::recommended_watcher   ◀── watches rootUri recursively
   │
   ▼
  notify-debouncer-mini (200ms per-path)
   │
   ▼
  worker thread:
     for each event path:
       if path ends in .tex: read → extract_labels(text, path, &index)
       if path ends in .bib: read → parse_bibtex(text, path, &index)
       // both parsers retain-then-insert internally; no separate remove_file call.
       // a failed read keeps the prior index entry (matches the LSP update_file
       // behaviour when the buffer is no longer on disk).
```

The watcher runs **inside the Rust sidecar**, not in Node. Rationale: it gives us the same reparse path as the LSP-driven `update_file`, with no extra IPC method, and Node is naturally oblivious — the next `lookup` just sees fresh data. Mirrors the plan §2 "what moves to Rust" table.

---

## 3. Data flow — ref hover end to end

1. User hovers over `\eqref{eq:foo}` at byte offset `O` of `paper/sections/intro.tex`.
2. `hoverFor` calls `sidecar.cursor_context(uri, O)`.
3. `cursor.rs::cursor_context` walks back from `O`; finds `\eqref{` enclosing the cursor. Returns `{ kind: "ref", key: "eq:foo", range: [a, b] }`.
4. `hoverFor` on `kind=ref` calls `sidecar.lookup("eq:foo", "ref")`.
5. `main.rs::handle_lookup` returns `{ found: true, entry: LabelRef { key, file, line, env, math, caption, snippet } }` or `{ found: false }`.
6. `refHoverFor`:
   - **found=false** → return `null`. Hover does not appear. (Matches latex-workshop convention; user opted for this.)
   - **found=true**:
     - build header from `env` + `caption` (or key when caption is empty)
     - append fenced code block of `snippet`
     - append `file:line`
     - return `{ contents: { kind: "markdown", value } }`. No SVG, no MathJax call.

`refHoverFor` runs entirely in TypeScript. The Rust side never sees the formatted output.

---

## 4. Component design

### 4.1 `latex-index/src/index.rs` — `LabelEntry` adds `snippet` field; extraction in `labels.rs`

**Struct change** (in `index.rs`, where `LabelEntry` already lives — see `index.rs:13`):

```rust
pub struct LabelEntry {
    // ... existing fields ...
    /// Source-code snippet around the label, pre-formatted for hover.
    pub snippet: String,
}
```

**Extraction logic** (in `labels.rs`, inside `extract_labels` at the existing `\label` handling site):

- **Math env** (`equation`, `align`, `gather`, `multline` and their starred forms): from the byte just past `\begin{env}` through the byte just before `\end{env}`. Trim leading and trailing blank lines; leave interior whitespace as-is.
- **Theorem-like env** (`theorem`, `lemma`, `proposition`, `corollary`, `definition`, `remark`, `example`, `claim`, `conjecture` and their starred forms): same body extraction as math.
- **Section** (`\section{Title}\label{…}`): the single line containing the `\section` command, with the trailing `\label{…}`.
- **Everything else** (free-floating `\label{…}`): the single line containing `\label{…}` plus one line of context before, total ≤ 2 lines.

**Truncation policy** — single rule, applied uniformly:

- If the extracted body is **≤ 12 lines AND ≤ 4 KiB**: use verbatim.
- If it exceeds either limit: take the first N lines that fit, and **append a single TeX-comment line** `% (truncated)` at the end.
- If we cannot locate a body boundary (e.g. unclosed `\begin{equation}`): fall back to the single line containing `\label{…}` and **append** `% (truncated)`. The trailing marker makes the truncation explicit in the rendered hover.

The snippet is **plain text with literal TeX** — no escape, no re-format. Markdown's triple-backtick fence accepts it as-is.

### 4.2 `latex-index/src/watcher.rs` — new file

```rust
pub fn spawn_watcher(
    root: PathBuf,
    index: Arc<Index>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut debouncer = new_debouncer(
            Duration::from_millis(200),  // debounce window
            Duration::from_millis(100),  // tick rate
            move |res: DebounceEventResult| match res {
                Ok(events) => for e in events { handle_event(&index, &e.path); },
                Err(e) => eprintln!("watcher: {e}"),
            },
        );
        debouncer.watcher().watch(&root, RecursiveMode::Recursive);
        // Block until the debouncer is dropped (i.e. main process exits).
        loop { std::thread::park_timeout(Duration::from_secs(3600)); }
    })
}
```

`handle_event`:
- if path's extension is not `.tex` or `.bib`: drop it.
- filter out paths whose ancestor chain contains any of `crate::workspace::SKIP_DIRS`.
- read the file (`fs::read_to_string`); on error, **return silently** — the prior index entry stays. (Both `extract_labels` and `parse_bibtex` already do `retain` on entry, so a successful read also drops the prior entry for that path as a side effect.)
- dispatch on extension:
  - `.tex` → `labels::extract_labels(&text, &path, &index)`
  - `.bib` → `bibtex::parse_bibtex(&text, &path, &index)`

**Configuration**: the 200 ms debounce window is hardcoded for v1. Exposing `watchDebounceMs` as a user config is a Phase 3 polish item — it does not block Phase 2 because the chosen value (200 ms) sits in the sweet spot where editor-save bursts are absorbed but interactive edits still feel live.

**`SKIP_DIRS` reuse**: the watcher imports `use crate::workspace::SKIP_DIRS;` rather than re-declaring. This avoids drift with `workspace.rs:13` (which is the canonical list and includes `.vscode`/`.idea`/`.texghost` that `preamble.ts:197` historically missed). The mismatch between Rust and the Node fallback is a known wart, but Phase 2 does not touch it; this spec only reuses the Rust canonical.

We intentionally do **not** parse `.gitignore` here. The `ignore` crate pulls in a lot; the value is marginal for a latex preview tool. If a user reports false-positive scans we revisit.

### 4.3 `latex-index/src/main.rs` — wire watcher, share `Index`

Three surgical edits:

1. **State field**: `index: Index` → `index: Arc<Index>`. Existing call sites pass `&state.index` to `extract_labels`/`parse_bibtex`/`remove_file`; those keep working unchanged via `Arc<T>` Deref to `&T`.
2. **Construction**: `index: Index::new()` → `index: Arc::new(Index::new())` in `State::new`.
3. **Spawn watcher after `handle_initialize`**: in `handle_initialize`, after the protocol version check, stash the `rootUri` (currently bound to `_root` and discarded) on `State` and call `watcher::spawn_watcher(root_path, state.index.clone())`. Two guards:
   - **Null root**: if `rootUri` is `None` or fails `normalise_uri`, do **not** spawn the watcher. Phase-1 tests pass `rootUri: null` and must still pass.
   - **Idempotent init**: if `state.initialised` is already `true`, skip both the response and the spawn. LSP clients are expected to call `initialize` exactly once, but a defensive second-call guard is cheap and removes a class of "watcher leaks a thread" bugs.
4. **Hold the JoinHandle**: stash the handle on `State` (e.g. `watcher: Option<JoinHandle<()>>`) so the OS thread is joined cleanly on `shutdown`. A leaked thread would survive an early return from `main`.

`state.index.clone()` clones the Arc, not the underlying map. The watcher can now mutate the map concurrently with the main loop's reads (DashMap handles the synchronisation; we are not introducing a new lock).

### 4.4 `latex-index/Cargo.toml` — new dependencies

```toml
[dependencies]
notify = "6"
notify-debouncer-mini = "0.4"
```

Both are small, well-maintained, no transitive surprises. `notify-debouncer-mini` re-exports a `DebounceEventResult` over `mpsc::Sender<DebouncedEvent>`; we pass a closure callback instead, which keeps `main.rs` loop changes minimal.

**No `tempfile` dependency** — `latex-index/src/workspace.rs:208` already exposes a `tempdir()` test helper (and a `cleanup` companion) inside the existing `#[cfg(test)] mod tests`. The new watcher tests reuse it.

**`Cargo.lock` regeneration is mandatory** — CI uses `cargo build --release --locked` (see `.github/workflows/ci.yml:55` and `:125`). The new `notify*` entries must be committed in the same PR. The implementation step is: `cargo build` locally once → `git add Cargo.lock` → commit. Without this, CI fails on the `--locked` check.

### 4.5 `server/src/rpc_types.ts` — extend `LabelRef`

```ts
export interface LabelRef {
  key: string;
  file: string;
  offset: number;
  line: number;
  env: string;
  math?: [number, number] | null;
  caption: string;
  /** Pre-formatted source snippet for hover rendering. */
  snippet: string;
}
```

Field is additive — old callers (none in production) that ignore the new field are unaffected. Phase 1 already tests this shape against the live binary, so the new field will show up the moment `LabelEntry` starts serialising it.

### 4.6 `server/src/ref_hover.ts` — real implementation

```ts
export function refHoverFor(
  result: RefLookupResult,
  range?: [number, number],
): { contents: { kind: "markdown"; value: string } } | null {
  if (!result.found) return null;
  const e = result.entry;
  const header = headerFor(e);                       // "Equation", "Theorem: …", "Section: …"
  const snippet = fence(e.snippet);                  // ```latex … ```
  const location = `${relativePath(e.file)}:${e.line + 1}`;
  const value = [header, snippet, location].filter(Boolean).join("\n\n");
  return { contents: { kind: "markdown", value } };
}
```

- `headerFor`: switch on `entry.env` — see §3 step 6 for the rules.
- `fence`: wrap the snippet in a CommonMark code fence. **Rule** (CommonMark §4.5): scan the snippet for the longest run of consecutive backticks; let that length be `N`. Use a fence of `N+1` backticks, and a matching closing fence of the same length. With `N=0` (no backticks in the snippet) this degenerates to the conventional triple-backtick fence. TeX almost never contains triple-backticks, but `lstlisting` / `minted` / `verbatim*` body text might, and the commonmark.org spec is explicit that an inner fence must be longer than any inner run. Implementation: one helper `fenceFor(snippet: string): string` that returns the opening fence (closing is identical). No per-character escaping required.
- `relativePath`: strip the LSP root prefix so the pointer is short.
- No SVG, no MathJax — this is documented in the plan and the user confirmed.

### 4.7 `server/src/hover.ts` — minimal change

The `ref` branch already exists (line ~95); the only delta is to pass the `range` through to `refHoverFor` so future enhancements (highlighting) can use it. Today the `range` is forwarded but unused inside `refHoverFor`.

### 4.8 `latex-index/src/labels.rs::REF_COMMANDS` — already correct

`["ref", "eqref", "cref", "Cref", "autoref", "nameref", "pageref"]` matches the seven commands the user gets to use. `cursor.rs` and `lookup("ref")` already route all of them.

### 4.9 Hover: package / command documentation (new)

A third small hover kind: when the cursor is on `\usepackage{<pkg>}` (specifically the `pkg` argument) or on a bare command name like `\textbf`, show a short description. Sourced from a bundled dictionary inside the Rust sidecar — no `texdoc` shell-out, no network.

**Data shape (in Rust, new file `latex-index/src/dict.rs`):**

```rust
pub struct DictEntry {
    pub title: String,         // "amsmath", "amssymb", "textbf", "textit", …
    pub kind: DocKind,         // Package | Command
    pub short: String,         // ≤ 200 chars
    pub docs: Option<String>,  // optional longer markdown body (≤ 2 KiB)
}

pub fn lookup(name: &str) -> Option<&'static DictEntry>;
```

A `static` `phf::Map` (or a sorted slice + binary search) initialised at startup. Hand-curated entries for ~30 common packages (`amsmath`, `amssymb`, `amsfonts`, `mathtools`, `bm`, `siunitx`, `booktabs`, `graphicx`, `hyperref`, `tikz`, `xcolor`, `listings`, `minted`, `algorithmicx`, `algorithm2e`, `glossaries`, `biblatex`, `natbib`, `geometry`, `caption`, `subcaption`, `inputenc`, `fontenc`, `lmodern`, `kpsewhich`, `xparse`, `etoolbox`, `tcolorbox`, `standalone`, `pdfpages`) and ~80 common commands (font, sectioning, math, references). Total: small fixed-size payload, no I/O.

**New IPC method:**

| Method        | Params                       | Returns                                                |
|---------------|------------------------------|--------------------------------------------------------|
| `doc_lookup`  | `{ name: string }`           | `{ found: bool, entry?: { kind, title, short, docs? } }` |

Lives next to `cursor_context` and `lookup` in the dispatcher. Returns `{ found: false }` for unknown names so Node can fall through cleanly (no hover, no error).

**Cursor-side dispatch (in `cursor.rs`):**

`cursor_context` gets a fourth `kind` value: `"doc"`. Detection rules:

- `\usepackage[<opts>]{<name>}` / `\usepackage{<name>}` / `\RequirePackage{<name>}`: when offset is inside the `name` braced group.
- Bare command: when offset is on a backslash-command name (e.g. `\text|bf`) and the command is in the dict.
- `\begin{<env>}` / `\end{<env>}`: same as bare command for envs the dict covers.

Other commands (e.g. `\frac`) fall through to `kind=none` — the dict is intentionally small, and `\frac` already has a good math hover.

**Node side (new `server/src/doc_hover.ts`):**

```ts
export async function docHoverFor(
  name: string,
  sidecar: SidecarHandle,
): Promise<HoverResult | null> {
  const r = await sidecar.doc_lookup(name);
  if (!r.found) return null;
  const e = r.entry;
  const header = `**${e.title}** (${e.kind})`;  // "**amsmath** (package)"
  const body = e.docs ? `\n\n${e.docs}` : `\n\n${e.short}`;
  return { contents: { kind: "markdown", value: header + body } };
}
```

Wired into `hover.ts` as a fourth dispatch branch after the existing `cite` / `ref` / `math`. Order matters: `cite` and `ref` first, then `doc`, then fall through to math.

**Out of scope (Phase 2):**
- `texdoc` shell-out (the dict is static; we never touch the filesystem at hover time).
- Per-user / per-project custom entries (a future `latex-preview.dict.json`).
- Hovering on the package options (`\usepackage[utf8]{inputenc}` — the `utf8` part). The `kind=doc` detection only fires on the package name's braced group.

---

## 5. Multi-key refs

`\cref{eq:a,eq:b,eq:c}` is a single command with three keys. The user opted for **"show only the one the cursor is on"**.

Implementation: `cursor.rs::detect_braced_command_at` returns the full key string verbatim (`"eq:a,eq:b,eq:c"`). `lookup` does not match (no label has that as its key) and returns `found=false` → hover does not appear. This is the "do nothing" path and matches the user's decision.

A future improvement: split on `,` inside the `cursor_context` call and return three separate `CursorContext` results. Not in this phase.

---

## 6. Error handling & edge cases

| Situation                                            | Behaviour                                                                    |
|------------------------------------------------------|------------------------------------------------------------------------------|
| `\ref` to a non-existent label                       | `found=false` → no hover.                                                     |
| `\ref` to a section label                            | Plain markdown, no SVG; header is `Section: <caption>`.                      |
| `\ref` to a theorem label                            | Header is `Theorem: <caption>` (or "Lemma: …" etc. derived from `env`).      |
| `\ref` to a math label (equation / align / …)        | Header is the env name only: `Equation`, `Align`, `Gather`, `Multline`, `…` — no number, since `LabelEntry` has no numbering field and `best_caption_for_env` (`labels.rs:389`) only fills captions for theorem envs. |
| Label inside a verbatim/comment                      | Already excluded by `extract_labels` (it doesn't see inside `\verb…\verb`).   |
| Snippet extraction can't find a body boundary        | Fall back to the single `\label` line; append `% (truncated)`.                |
| Snippet body exceeds 12 lines or 4 KiB               | Take the first N lines that fit, then append `% (truncated)`.                |
| Watcher: `.tex` / `.bib` file disappears (deleted on disk) | `fs::read_to_string` returns `Err`; silently keep the prior index entry. Next write wins. |
| Watcher receives a `.aux` / `.log` / `.out` change   | Extension filter rejects it before `handle_event` runs.                       |
| Watcher event for a path under `node_modules`        | Ancestor filter rejects it.                                                   |
| Two `update_file` events for the same path back-to-back | Debouncer keeps the latest, fires once at T+200ms.                         |
| Sidecar restarted mid-session                        | Watcher dies with the process. `startSidecar` reconnect does not currently re-spawn it — out of scope (Phase 1 close-out). |
| `rootUri: null` in `initialize`                      | Watcher not spawned; hover/lookup paths keep working.                        |
| `initialize` called twice                            | Second call is a no-op (idempotency guard); watcher is not double-spawned.    |
| File watch path > 260 chars on Windows               | `notify` handles long paths via `\\?\` prefix on Windows 10+. No special action. |

---

## 7. Testing strategy

### 7.1 New Rust unit tests (`latex-index/src/labels.rs`)

- `snippet_for_equation_includes_body`
- `snippet_for_theorem_captures_first_line`
- `snippet_for_section_is_single_line`
- `snippet_truncated_at_12_lines`
- `snippet_truncated_at_4_kib`

### 7.2 New Rust unit tests (`latex-index/src/watcher.rs`)

Use the existing `tempdir()` helper at `latex-index/src/workspace.rs:208` (and the `cleanup` companion) — no new `tempfile` crate:

- `filter_skips_noise_dirs` — events from `<tmp>/node_modules/foo.tex` are dropped by the ancestor check.
- `debounces_rapid_changes` — fire 5 writes within 50ms; assert the debouncer produces a single event.

### 7.3 New integration test (`latex-index/tests/end_to_end_ipc.rs`)

Add to the file (currently `#[ignore]`-d on Windows due to libtest output capture, but works on Linux/macOS):

- `external_change_picks_up_new_label` — spawn sidecar, send `initialize` with a real `rootUri` pointing at a `tempdir`, then write a temp `.tex` to that directory, wait 300ms (debounce + buffer flush), `lookup` the new entry, assert `found=true`.

**Required helper**: a new `initialize_with_root(s: &mut Sidecar, root: &Path)` that mirrors the existing `initialize` (line 162) but passes `rootUri: format!("file://{}", root.display())` instead of `null`. The existing `initialize` stays as the "no watcher" path; the new helper exercises the watcher.

### 7.4 New Node test file (`server/test/ref_hover.test.ts`)

Eight tests:

- `found_renders_equation_snippet`
- `found_renders_theorem_with_caption`
- `found_renders_section_as_plain_markdown`
- `found_renders_label_outside_env`
- `not_found_returns_null`
- `snippet_with_triple_backtick_is_escaped`
- `snippet_truncated_marker_appears_when_oversize`
- `header_for_unknown_env_falls_back_to_key`

These run inside the existing `npm test` invocation; no new infra.

### 7.5 Regression safety

The 25 existing Rust unit tests + 4 ignored integration tests + 104 Node tests all remain untouched. Any drop in their pass count is a regression this spec explicitly forbids.

**Scope reminder.** This repository implements **only** `textDocument/hover` (and a file watcher to keep the hover data fresh). It does not register `textDocument/definition`, `textDocument/completion`, `textDocument/documentSymbol`, `textDocument/formatting`, or any `textDocument/codeAction` provider. LSP capabilities advertised in `server/src/server.ts` remain `{ textDocumentSync: Full, hoverProvider: true }` and nothing more. Features that would normally fall under those handlers — goto-definition for labels, `\cite{` completion, document outline — are explicitly out of scope and intentionally delegated to `zed_latex` (the full LaTeX plugin). See the repository `README.md` for the boundary statement.

### 7.6 Performance budget

- `\ref` hover latency: < 1 ms TS-side formatting + 1 IPC round-trip ≈ 5 ms p50, < 20 ms p99. (Plan §9 AC4.)
- Watcher steady-state: 0 events when no files change. CPU during heavy editor save (200ms debounce, single reparse per path): << 1ms.
- No new disk I/O on the LSP hot path. The watcher's read is a one-shot on a 4 KiB file.

---

## 8. Rollout

Phase 2 ships in one PR. Branch: `phase2-ref-hover-watcher`. Commits in this order (each individually builds & tests pass):

1. **`feat(rust): add snippet field to LabelEntry` (+ unit tests).** Struct change in `index.rs`, extraction in `labels.rs`. Cargo.lock regenerated.
2. **`feat(rust): add file watcher with debounce` (+ unit tests).** New `latex-index/src/watcher.rs`. `Arc<Index>` plumbing in `main.rs`. Idempotency + null-root guards.
3. **`chore: extend rpc_types.ts LabelRef with snippet` (+ `doc_lookup` types).** Pure type addition; no behaviour change.
4. **`feat(server): implement ref_hover.ts and doc_hover.ts` (+ tests).** `fenceFor` helper, `headerFor` switch, `doc_hover.ts` for the new `kind=doc` branch, `hover.ts` wires the fourth dispatch.
5. **`test: integration test for watcher picking up external changes`.** Uses `initialize_with_root` helper.
6. **`docs: Phase 2 design spec` (this file).** Committed last so reviewers see code + design together.

The order matters: steps 3 and 4 depend on step 1's field; step 4 depends on step 3's type; step 5 depends on step 2's spawn semantics; step 6 is reference material for the others.

The integration test in 7.3 runs on Linux + macOS CI (already configured in `.github/workflows/ci.yml`). Windows CI is unaffected — the test is `#[ignore]`-d as before.

Phase 1 close-out items (async `workspace_macros`, sidecar restart, macos-arm64 runner) remain blocking for "Phase 1 complete" but are not in this PR. They are unblocked by no new code here.

---

## 9. Open questions deferred to Phase 3

All Phase 3 candidates stay inside the **hover-only** scope (see §7.5 scope reminder and the repository `README.md`):

1. **Multi-key `\cref` / `\cite` split.** `\cref{eq:a,eq:b}` and `\cite{a,b,c}` are currently one-command-many-keys; the cursor hits one key, the others fall through with `found=false`. Split the key list at `,` in `cursor.rs` and emit one hover per visible key.
2. **Bibtex entry richer preview.** Render the `abstract` field in the cite hover, plus optional `doi` / `url` as clickable links (markdown links, no LSP `documentLink`).
3. **Dictionary expansion.** Grow `latex-index/src/dict.rs` from ~130 entries toward ~300 — more packages (`siunitx`, `tikz`, `beamer`, `hyperref`, `listings`, `minted`, `fontspec`, `unicode-math`, `physics`, `bm`, `mathtools`, `biblatex`, `natbib`, `geometry`, `caption`, `subcaption`, `xcolor`, `inputenc`, `fontenc`, `lmodern`, `etoolbox`, `xparse`, …) and more commands (math + sectioning + reference). Source: hand-curated, no `texdoc` shell-out.
4. **WatchDebounceMs configuration.** Hardcoded to 200ms in §4.2; expose as a user setting in a later phase.
5. **`.gitignore` parsing for the watcher scope.** Marginal value; revisit only on user complaint.
6. **Diagnostic on `found=false`.** Currently hover silently does not appear. Could publish a `Diagnostic` with `severity: Hint` instead, but that crosses into `textDocument/publishDiagnostics` territory — only do it if the hover-only boundary is reconsidered.

**Explicitly not on the Phase 3 list** (delegated to `zed_latex`):
- `textDocument/definition` for labels / cites / packages
- `textDocument/completion` for `\cite{` / `\ref{` / `\usepackage{`
- `textDocument/documentSymbol` / outline
- `textDocument/formatting` (latexindent integration)
- `textDocument/codeAction` / chktex diagnostics
- `textDocument/documentLink`

Snippet truncation policy (12 lines / 4 KiB, append `% (truncated)`) is fixed in §4.1. None of the items above block this spec.
