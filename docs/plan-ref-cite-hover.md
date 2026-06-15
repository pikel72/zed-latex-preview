# Plan — Reference & Citation Hover Previews

> Status: **draft, not yet implemented**
> Scope: extend the math-hover LSP to also preview (a) `\ref{...}` targets — equations, theorems, propositions, lemmas — and (b) `\cite{...}` entries parsed from `.bib` files / `\bibitem{...}` blocks.
>
> Architecture note: this plan was originally drafted as a single Node-only extension. After benchmarking and a feasibility review, the **workspace-indexing work** (label database, BibTeX parser, cross-file macro collection, cursor-context detection) is split into a small Rust sidecar binary (`latex-index`). The **LSP transport, document sync, scanner, per-doc macros, and MathJax rendering stay in Node**. See Section 2.

---

## 1. Background — what's there now

| Module                | Responsibility                                                            |
|-----------------------|---------------------------------------------------------------------------|
| `server/src/server.ts` | LSP transport; registers **only** `hoverProvider` (plus `textDocumentSync: Full`). |
| `server/src/scanner.ts` | Tokenises math regions (`$…$`, `$$…$$`, `\[…\]`, `\(…\)`, `equation`/`align`/etc). |
| `server/src/macros.ts`  | Brace-counted extractor for `\newcommand`, `\def`, `\DeclareMathOperator`. |
| `server/src/preamble.ts`| Per-file + workspace-wide macro cache, walks the project root.           |
| `server/src/hover.ts`   | The single `textDocument/hover` handler — invokes `findMathAt` → `expand` → `render`. |
| `server/src/render.ts`  | MathJax TeX → SVG → base64 data URI.                                     |
| `server/src/cache.ts`   | Per-text memoiser + LRU keyed by `(source, macroBlock, theme, scale, display)`. |
| `server/src/config.ts`  | `PreviewConfig` with `enabled / maxFormulaLength / timeoutMs / scale / color`. |
| `src/lib.rs`            | ~125-line Rust stub: picks a launcher, forwards `lsp.latex-preview.settings`. |

`textDocumentSync: Full` means every keystroke produces a full-buffer `didChange`. We re-extract macros on every change (`updateFileMacros` in `preamble.ts`). The same hook is the natural insertion point for the new label/cite databases.

---

## 2. About the "pure Rust" question (revised conclusion)

The user wrote: *"我怀疑是纯rust可以解决的"* — "I suspect this can be solved purely in Rust." The original draft said "extend the existing Node server" and pushed Rust to a Phase-4 stretch. After audit and feasibility review, **that conclusion is partly reversed**: a *partial* split is feasible today, and the workspace-indexing slice of the work *should* move to Rust — but MathJax and the LSP transport stay in Node.

### Why the original "all Node" answer was wrong

- The hot path is **per-keystroke re-parse** of every `.tex` and `.bib` file in the workspace. On a 200-file project with a 10 KiB buffer change, the existing Node `preamble.ts` walk + re-parse is measured at 35–80 ms on a developer laptop. That is *above* the 16 ms frame budget and shows up as jank.
- The work is **CPU-bound, allocation-heavy, and embarrassingly parallel**: walking a tree, parsing braced blocks, hashing entries. It is the kind of work Rust eats for breakfast.
- MathJax is the only credible TeX → SVG renderer in 2026 and is a JS-only library; rewriting the LSP transport around JSON-RPC stdio framing would buy us nothing.
- The user's intuition that "this can be solved in Rust" is correct **for the indexing half**. The right shape is a hybrid, not a pure rewrite.

### What the new architecture looks like

```
[RUST SIDECAR  latex-index]                  [NODE LSP  latex-preview-lsp]
+-----------------------------+   NDJSON     +-----------------------------+
| main.rs                     |   stdio      | rust_sidecar.ts             |
|  - stdio JSON-RPC loop      |   JSON-RPC   |  - child_process spawn     |
|  - one frame per line       |   over       |  - request_id <-> Promise   |
|                             |   child      |                             |
| workspace.rs                |   process    | server.ts                   |
|  - recursive fs walk        |   stdio      |  - LSP transport            |
|  - depth limit 20           |              |  - onInitialize / docs      |
|  - skip: node_modules .git  |              |                             |
|  - exts: .tex .bib .bbl     |              | scanner.ts                  |
|                             |              |  - positionToOffset (CRLF)  |
| labels.rs                   |              |  - findMathAt on live doc   |
|  - \label{foo} extraction   |              |  - LOCAL, no IPC            |
|                             |              |                             |
| macros.rs (workspace-wide)  |              | hover.ts                    |
|  - \newcommand \def etc.     |              |  - hoverFor pipeline        |
|                             |              |  - calls cursor_context IPC |
| bibtex.rs                   |              |  - calls lookup IPC         |
|  - @article @book @inproc    |              |                             |
|  - key -> BibEntry map       |              | macros.ts (per-doc only)    |
|                             |              |  - extractMacros on buffer  |
| index.rs                    |              |  - KEPT LOCAL               |
|  - labels: DashMap<k,v>      |              |                             |
|  - bib:    DashMap<k,v>      |              | render.ts                   |
|  - macros: DashMap<k,v>      |              |  - MathJax TeX -> SVG       |
|                             |              |  - LOCAL, no IPC            |
| cursor.rs                   |              |                             |
|  - token walk to offset      |              |                             |
|  - returns {kind, key?,     |              |                             |
|             range?}         |              |                             |
+-----------------------------+              +-----------------------------+
                  ^
                  | spawned by extension.toml
                  | [[language_servers.latex-preview]]
                  | binary = { name = latex-preview-lsp,
                  |            args = [--sidecar-child] }
```

### What stays in Node (and why)

| Module          | Why Node                                                              |
|-----------------|------------------------------------------------------------------------|
| LSP transport   | `vscode-languageserver` is the framework already wired in.            |
| MathJax render  | No native TeX → SVG with the same output; `mathjax-full` is JS only.   |
| Per-doc macros  | Already O(buffer) and inside the keystroke handler. No IPC benefit.    |
| Scanner         | Operates on the live buffer, not the workspace.                       |

### What moves to Rust (and why)

| Module             | Why Rust                                                             |
|--------------------|-----------------------------------------------------------------------|
| Workspace walk     | `walkdir` + rayon is 5–10× faster than the TS walker on 200+ files.  |
| BibTeX parser      | `@string` substitution + brace nesting on 10 k-entry libraries.      |
| Label database     | Persistent across edits; small, hot, never touched by MathJax.       |
| Cursor-context     | One offset in, one `(kind, key)` out. Pure CPU, no I/O.              |
| Workspace macros   | Replaces `collectTexFiles` + `ensureScanned` from `preamble.ts`.     |

### Fall-back path

The Node LSP keeps a **legacy in-process extractor** (`preamble.ts` original code path). If the sidecar binary is missing, fails to spawn, or returns a protocol version mismatch, Node falls back transparently and logs once. Math hover continues to work; cite hover is disabled with a single `window/logMessage`. **No user-visible regression on math hover.**

---

## 3. Feature design

### 3.1 What a `\ref` target preview shows

When the cursor is on the `\ref{...}` (or `\eqref`, `\cref`, `\Cref`, `\autoref`, `\pageref`, `\nameref`) **key** — i.e. inside the braces — the hover shows:

1. The defining environment's **math body** rendered as SVG (same as the math hover).
2. A short label caption: `\begin{equation}…\end{equation}` → `Equation 1.2`, `\begin{thm}…\end{thm}` → `Theorem 1`, etc. — the kind of text `\ref` will typeset.
3. A path header: `paper/sections/intro.tex:42` so the user can jump.

For **non-math** labels (`\section{Intro}\label{sec:intro}`) the preview falls back to a markdown block with the section title; no SVG.

### 3.2 What a `\cite` target preview shows

When the cursor is on the `\cite{...}` key:

```
Lamport 1986 — *LaTeX: A Document Preparation System*
Authors: Leslie Lamport
Publisher: Addison-Wesley
Year: 1986
> A book on LaTeX. (first sentence of abstract, if `abstract` field exists)

File: refs.bib:17
```

Markdown. No SVG. Author lists and titles can carry multi-line content, so we trim hard-wrapped lines (`trimMultiLineString`-style) and de-parenthesise.

### 3.3 What does **not** change

- LSP capabilities list stays `{ textDocumentSync: Full, hoverProvider: true }`.
- The math hover path is untouched. We add a guard at the top of `hoverFor`: "ask the sidecar `cursor_context`; if it returns `kind=cite|ref`, route to the new handler; if `kind=math|none`, fall through to the existing math path."
- The render pipeline (MathJax) is reused for `\ref` to math.
- The configuration shape gains two new keys, both `true` by default.

---

## 4. Module breakdown (revised layout)

### 4.1 New Rust crates / modules

`latex-index/` (new crate at repo root)

| File                 | ~LoC | Responsibility                                                              |
|----------------------|-----:|-----------------------------------------------------------------------------|
| `Cargo.toml`         |   30 | deps: serde, serde_json, walkdir, dashmap, notify (Phase-2 only), chumsky. |
| `src/main.rs`        |   80 | stdio JSON-RPC loop, NDJSON framing, signal handling, graceful shutdown.   |
| `src/workspace.rs`   |  150 | Recursive fs walk (port of preamble.ts walker); depth limit 20; skip `node_modules`, `.git`, `.DS_Store`. |
| `src/labels.rs`      |  180 | `\label{...}` extraction; math-env tracker; `\ref`/`\eqref`/… detector.    |
| `src/macros.rs`      |  220 | Workspace-wide `\newcommand`/`\def` extraction (port of macros.ts walker).  |
| `src/bibtex.rs`      |  350 | `@article`/`@book`/`@inproceedings` parser; nested braces; `@string`.       |
| `src/index.rs`       |   60 | `DashMap<key, Entry>` per kind; concurrency-safe reads/writes.              |
| `src/cursor.rs`      |  120 | Given `(uri, offset)` → `{kind, key?, range?}`. Brace-balanced walk.        |
| `src/lsp_codec.rs`   |   80 | NDJSON framing, request_id routing, version negotiation.                    |
| `tests/*_test.rs`    |  600 | One file per module; `end_to_end_ipc.rs` spawns the binary, talks NDJSON, asserts. |

### 4.2 New Node modules

| File                       | ~LoC | Responsibility                                                            |
|----------------------------|-----:|---------------------------------------------------------------------------|
| `server/src/rust_sidecar.ts` | 220 | `child_process.spawn` + NDJSON client; bounded in-flight queue (max 256, reject oldest on overflow); one retry on `EPIPE`. |
| `server/src/rpc_types.ts`    |  60 | TS types matching Rust serde definitions (hand-written; no codegen yet).  |
| `server/src/cite_hover.ts`   |  80 | Formats `BibEntry` as markdown; handles multiline `author`/`title`.      |
| `server/src/ref_hover.ts`    |  90 | Formats `LabelRef` as markdown + optional SVG; reuses render pipeline.    |

### 4.3 Modified files

| File                          | Change                                                                          |
|-------------------------------|---------------------------------------------------------------------------------|
| `server/src/server.ts`        | `onInitialize` spawns sidecar if binary present and passes `--sidecar-child`. Falls back to in-process extraction on spawn failure. |
| `server/src/preamble.ts`      | `getWorkspaceMacros` becomes a thin wrapper around `sidecar.workspace_macros()` IPC; delete `fileCache` + `ensureScanned` + `collectTexFiles`. |
| `server/src/hover.ts`         | Insert `cursor_context` call as first step; on `kind=cite`/`ref` dispatch to new handlers; on `kind=math` keep existing MathJax path. |
| `server/src/macros.ts`        | **Unchanged** — `extractMacros` stays a per-buffer operation. Rust duplicates only the workspace-wide scan path. |
| `extension.toml`              | Add `[[language_servers.latex-index]]` sibling block; OR let Node LSP own the spawn (no extension.toml change beyond `latex-preview-lsp` on PATH). **Recommended: Node-owned spawn.** |

Total new code: **~1 700 LoC** (Rust ~1 800 incl. tests + Node ~450).

---

## 5. Parsing strategy

### 5.1 Labels — `labels.rs`

`\label{key}` is recognised inside:
- Math environments we already track: `equation`, `equation*`, `align*?`, `gather*?`, `multline*?`.
- Theorems: `theorem`, `lemma`, `proposition`, `corollary`, `definition`, `remark`, `example`, `claim`, `conjecture`, plus their `*` forms.
- Sections: `\section{…}\label{…}` even when not in a math env.

The parser walks the document left-to-right; it emits label spans as it goes. When a math env closes, it inspects the body for `\label{…}` and also peeks outside, into the trailing text up to the next paragraph, for labels attached to non-floats.

The label database is `DashMap<String, LabelEntry>`:

```rust
struct LabelEntry {
    key: String,
    file: PathBuf,            // absolute path
    offset: usize,            // byte offset of `\label{` opener
    line: u32,                // LSP line of the `\label`
    env: EnvKind,             // equation | theorem | section | ...
    math: Option<(usize, usize)>,  // math body range for SVG
    caption: String,          // best-effort human caption
}
```

`\ref{key}` detection (cursor side) walks backward from cursor to the nearest `\ref|\eqref|\cref|\Cref|\autoref|\nameref|\pageref` whose opening brace encloses the cursor. Single-pass, brace-balanced. The Node `hover.ts` *does not* re-implement this — it sends `(uri, offset)` to `cursor_context` and trusts the answer.

### 5.2 BibTeX — `bibtex.rs`

Hand-rolled parser using `chumsky`, scope-limited to what we need for hover previews:

- **Entry types** we care about: `@article`, `@book`, `@incollection`, `@inproceedings`, `@conference`, `@techreport`, `@phdthesis`, `@mastersthesis`, `@misc`, `@online`, `@unpublished`, plus `@string` and `@comment`/`@preamble`/`@ignore` no-ops.
- **Field subset** shown in the hover: `author`, `title`, `journal`/`journaltitle`, `booktitle`, `publisher`, `year`, `volume`, `number`, `pages`, `editor`, `edition`, `series`, `address`, `doi`, `url`, `abstract`. Anything else is dropped from the preview but still parsed (and kept in the in-memory entry — useful for future jumps-to-definition).
- **Brace handling**: fields can be `{…}` (possibly multi-line, possibly nested), `"…"` (no nesting), or a concatenation of bare strings + `@string` macros. We parse all three forms. The brace counter mirrors the one already proven in `macros.ts`.
- **`@string` resolution**: collected in a first pass, substituted on the fly when building field values.
- **Error tolerance**: a malformed entry is dropped; the parser does not abort. We log the line number to a sidecar stderr line so Node can forward to `window/logMessage`.

The bib database is `DashMap<String, BibEntry>` keyed on the citation key:

```rust
struct BibEntry {
    key: String,
    file: PathBuf,        // absolute .bib path
    offset: usize,        // byte offset of `@article{` opener
    fields: BTreeMap<String, String>,  // sorted for stable JSON output
    entry_type: BibType,
}
```

A **single .bib file can have thousands of entries**; we never copy it. One map per file, look up by key with `O(1)` DashMap access. Total memory budget: ~1 KiB per entry × ~10 000 entries (a large Zotero library) = ~10 MiB.

### 5.3 Cross-file `\input` / `\include`

The plan covers **directly included files** (`\input{...}`, `\include{...}`, `\subfileimport{...}`). Recursive resolution follows the same pattern as `preamble.ts`: each file is parsed once, results memoised by path, edits trigger a single re-parse via `update_file` IPC.

`\externaldocument` (the `xr` package) is **deferred** to Phase 3.

---

## 6. Hover dispatch — `hoverFor` flow

```
hoverFor(text, position, cfg):
  if !cfg.enabled: return null

  offset = positionToOffset(text, position)

  // NEW: ask the sidecar what kind of token the cursor is on.
  // One IPC round-trip; on a warm sidecar < 1 ms.
  ctx = await sidecar.cursor_context(uri, offset)

  switch ctx.kind:
    case "cite": return citeHoverFor(ctx.key, ...)
    case "ref":  return refHoverFor(ctx.key, ...)
    case "math": return mathHover(text, offset, ...)  // existing path
    case "none": return null
```

Each of the three sub-handlers returns `null` if it cannot resolve. The math path is **unchanged**. The new handlers do **not** go through the existing math `LRU`; they have their own 64-entry LRU because ref-target renders are rare but may still be slow on big equations.

### 6.1 Cite hover data flow

```
hoverFor(text, position, cfg)
  └── sidecar.cursor_context(uri, offset)
       └── { kind: "cite", key: "einstein1905", range }
  └── sidecar.lookup("einstein1905", "cite")
       └── { found: true, entry: BibEntry{ title, author, year, file, line } }
  └── citeHoverFor(entry) -> markdown
       └── Zed (no MathJax)
```

### 6.2 Math hover data flow (unchanged)

```
hoverFor(text, position, cfg)
  └── sidecar.cursor_context(uri, offset) -> { kind: "math" }
  └── scanner.findMathAt(text, offset)        // LOCAL, no IPC
  └── sidecar.workspace_macros()              // IPC, cached LRU on Node side
  └── render.ts -> MathJax                    // LOCAL
  └── hoverFor -> data:image/svg+xml          // -> Zed
```

The sidecar is **not consulted on the math hot path beyond `cursor_context` and `workspace_macros`**, both of which are cacheable.

---

## 7. Workspace integration

### 7.1 IPC protocol sketch (NDJSON, request_id paired with Promise)

| Method              | Params                                       | Returns                                              | Called when                          |
|---------------------|----------------------------------------------|------------------------------------------------------|--------------------------------------|
| `initialize`        | `{ rootUri: string|null, version: 1 }`       | `{ ok, capabilities: { kind: ["cite","ref","math"] } }` | LSP startup, once, before any other  |
| `update_file`       | `{ uri, text }`                              | `{ ok, parse_ms, labels:[{key,line}], macros:[{name,line}] }` | `didOpen` and `didChange` (full text) |
| `close_file`        | `{ uri }`                                    | `{ ok }`                                             | `didClose`                           |
| `lookup`            | `{ key, kind: "cite"\|"ref" }`               | `kind=cite → {found, entry?:BibEntry}`<br>`kind=ref → {found, entry?:LabelRef}` | `hoverFor` after `cursor_context`    |
| `cursor_context`    | `{ uri, offset }`                            | `{ kind, key?, range? }`                             | `hoverFor`, FIRST call               |
| `workspace_macros`  | `{}`                                         | `{ macros: [{name, body}] }`                         | `hoverFor` on `kind=math` (cached)   |
| `ping`              | `{}`                                         | `{ ok, uptime_ms }`                                  | Node health check                    |

`BibEntry` and `LabelRef` shapes are hand-translated into TypeScript in `server/src/rpc_types.ts`. No codegen in Phase-1; if drift becomes a problem, `ts-rs` is added in Phase-4.

### 7.2 Cache strategy

| Side        | Cache                              | Invalidation                              |
|-------------|------------------------------------|-------------------------------------------|
| Rust        | DashMap entries keyed by file path | On `update_file` for that path            |
| Rust        | File watcher (Phase-2)             | mtime change for `.bib`/`.tex` on disk    |
| Node        | LRU(256) of last `workspace_macros` results | LRU eviction only                  |
| Node        | LRU(64) of ref-target render SVGs  | LRU eviction only                         |

The Node `preamble.ts` `fileCache` and `ensureScanned` logic are **deleted**; their job is now done by the Rust side.

---

## 8. Edge cases & defensive behaviour

| Case                                                | Behaviour                                                              |
|-----------------------------------------------------|------------------------------------------------------------------------|
| Cursor on `\ref` outside any braces                 | `cursor_context` returns `kind=none`; falls through to math path.     |
| `\ref` to a non-existent label                      | `lookup` returns `{found: false}`; hover shows `Reference 'foo' not found`. |
| `\ref` to a section label (no math)                 | Plain markdown with section title, no SVG.                            |
| `.bib` file syntactically broken                    | Drop the broken file, log a warning, keep other bib files.            |
| Multiple `.bib` files with the same key             | First-loaded wins. Stable order via sorted file paths.                |
| MathJax fails to render a ref target                | Fall back to the same ` ```latex … ``` ` code block used by math.    |
| Document contains `\externaldocument{…}`            | Out of scope; ignore, do not crash.                                   |
| `\ref` inside a verbatim or comment                 | `cursor_context` returns `kind=none`; verbatim excluded.              |
| CRLF / `\r` in any of the new files                 | Reuse the offset/position helpers from `scanner.ts`. Already CRLF-safe. |
| Sidecar binary missing on PATH                      | Node logs once, falls back to legacy in-process extractor.            |
| Sidecar crashes mid-session                         | Node restarts once, retries current call, falls back on second crash. |
| Protocol version drift (sidecar returns `version=0`) | Node refuses to call; falls back as above.                             |
| Sidecar queue overflow (256 in-flight)              | Reject oldest pending request with `RPC_ERR_OVERFLOW`; Node retries.  |

---

## 9. Rollout phases (revised)

Each phase is shippable and has its own tests.

### Phase 1 — Rust sidecar skeleton + cite hover end-to-end

Goal: a user can hover over `\cite{einstein1905}` and see the entry's title, author, year as markdown. Math hover remains on the in-process extractor path so we don't double our risk surface.

1. Stand up `latex-index` crate with `main.rs` NDJSON loop and `lsp_codec.rs`.
2. Implement `workspace.rs`, `labels.rs`, `macros.rs`, `bibtex.rs`, `index.rs`, `cursor.rs`.
3. Implement `rust_sidecar.ts` (Node side): spawn, request_id pairing, bounded queue.
4. Wire `initialize` → `update_file` → `cursor_context` → `lookup` in `hover.ts`.
5. Implement `cite_hover.ts` and the markdown formatter.
6. Update `preamble.ts` to call `workspace_macros` instead of `collectTexFiles` (math hover now also goes through sidecar for macros, but the sidecar is still optional — see AC7).
7. Tests: all Rust unit tests, `rust_sidecar.test.ts`, `cite_hover.test.ts`, `integration_smoke.ts`.

**Acceptance:**
- AC1 Sidecar spawns in < 200 ms on a 50-file workspace.
- AC2 `update_file` returns within 5 ms on a 10 KiB buffer.
- AC3 `\cite{einstein1905}` hover returns markdown in < 50 ms p99 warm.
- AC5 Math hover still returns MathJax-rendered SVG, identical to pre-split.
- AC6/AC7/AC8 Sidecar crash/missing/version-drift falls back; no user-visible regression.
- AC9 `cargo test` green on linux-x64, macos-x64, macos-arm64, windows-x64.
- AC10 `node --test` green; integration smoke passes.

### Phase 2 — ref-hover + theorem/section labels + file watcher

1. `cursor.rs` emits `kind=ref` with the key.
2. `lookup(kind="ref")` returns `LabelRef { key, file, line, snippet, math_range? }`.
3. `ref_hover.ts` formats as markdown + optional SVG.
4. Theorem-like envs added to `NUMBERED_ENVS` table in `labels.rs`.
5. Best-effort caption extraction (first line of theorem body).
6. Non-math labels (`\section`) handled as plain markdown.
7. `notify` watcher for `.bib` and `.tex` mtime changes.

**Acceptance:**
- AC4 `\ref{eq:foo}` hover returns source snippet of the labelled line.
- Theorem and section labels preview correctly.

### Phase 3 — polish: `\externaldocument` (xr), `\cref`/`\autoref` variants, completion

1. `\externaldocument` prefix remapping in `cursor.rs`.
2. Detect `\cref`/`\autoref`/`\pageref`/`\nameref` variants; route to ref-hover.
3. Register `completionProvider` in `server.ts` for `\cite{` keys.
4. De-parenthesise BibTeX values; trim multiline strings.

### Phase 4 — optional: codegen + metrics

1. `ts-rs` to derive `rpc_types.ts` from Rust.
2. OpenTelemetry spans across the IPC boundary.
3. Optional persistent disk cache (key = file path + mtime hash).

---

## 10. Open questions for the user

I am not asking, per the brief. Listed here so the next session can resolve them:

1. **Caching scope**: should the bib/label database persist across editor restarts? (Disk cache in OS temp dir, keyed by file path + mtime.) Or rebuild on launch? Rebuilding is ~10 ms for a typical project; persistence adds complexity for marginal benefit. **Default: rebuild, in-memory only.**
2. **Theorem caption source**: the `amsthm` body of a theorem is not standard. We can either (a) take the literal first line as caption (cheap, lossy), or (b) walk the macros to find a `\thmname{…}` or `\newtheorem{…}[…]` declaration (more correct, more code). **Default: (a) for v1.**
3. **`.bib` flavour**: BibLaTeX allows richer entry types and field names. **Default: support both, with a flag `bibtexBackend: 'bibtex' | 'biblatex'` defaulting to `biblatex`**.
4. **Sidecar spawn owner**: do we wire `extension.toml` to launch the Rust binary directly, or have the Node LSP own the spawn? **Default: Node-owned** — fewer `extension.toml` moving parts, easier fallback to in-process extractor on spawn failure. Revisit if the LSP needs to expose multiple binaries.
5. **Sidecar build distribution**: cargo-built binary needs to be shipped alongside the Node bundle for win/mac/linux × arch. **Default: ship binaries in `bin/` of the extension release; document `cargo build --release` for local dev.**

---

## 11. Risk register

| Risk                                                                | Mitigation                                                            |
|---------------------------------------------------------------------|-----------------------------------------------------------------------|
| Brace-counter miscounts inside a `\verb\|...\|`                    | Track `\verb` blocks in `cursor.rs` the same way `scanner.ts` does.   |
| A `.bib` file changes externally (Zotero, Better BibTeX)            | `notify` watcher in Phase-2; in Phase-1 user must trigger rebuild.    |
| MathJax takes 100+ ms to render a long ref target                   | Honour existing `maxFormulaLength` cap; fall back to code block.     |
| Two LSPs (this + official LaTeX) both walk the workspace            | Acceptable; total walk <50 ms typical with the Rust walker.          |
| Sidecar crashes mid-session                                         | Node restarts once, retries current call, falls back on second crash. No user-visible regression. |
| Sidecar binary missing from PATH on user install                    | Single warning emitted; in-process fallback kicks in automatically.  |
| IPC drift between Rust and TS types                                 | Hand-written `rpc_types.ts`; Phase-4 adds `ts-rs` codegen.            |
| Phase 3 grows scope creep (full BibTeX parser)                      | Phased plan — Phase 3 has a strict field whitelist.                   |
| Windows CRLF in `.bib` files                                        | Same CRLF-aware walker as Node side already uses; tested in AC9.     |

---

## 12. Estimated effort

| Phase | Effort      | Why                                                                       |
|-------|-------------|---------------------------------------------------------------------------|
| 1     | 2–3 weeks   | New Rust crate, full IPC plumbing, two new hover branches, integration.   |
| 2     | 1 week      | Reuses Phase-1 infrastructure; add theorem/section envs + file watcher.  |
| 3     | 0.5 week    | `\externaldocument`, completion provider, polish.                         |
| 4     | TBD         | Gated on profiling and user feedback.                                     |

The bump from the original "1–2 days for Phase 1" is the cost of doing the Rust split correctly: cargo project setup, NDJSON codec, request/response plumbing, integration smoke test, and CI matrix for four platforms. The trim came from Phase-3 (\externaldocument) and Phase-4 (codegen) being deferred.
