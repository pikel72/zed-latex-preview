# Plan — Reference & Citation Hover Previews

> Status: **draft, not yet implemented**
> Scope: extend the math-hover LSP to also preview (a) `\ref{...}` targets — equations, theorems, propositions, lemmas — and (b) `\cite{...}` entries parsed from `.bib` files / `\bibitem{...}` blocks.

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

## 2. About the "pure Rust" question

The user wrote: *"我怀疑是纯rust可以解决的"* — "I suspect this can be solved purely in Rust." Honest answer:

- The `src/lib.rs` stub is currently **only** a launcher. The actual LSP server is the bundled Node.js program under `server/`, ~1700 LoC, and depends on `mathjax-full` for SVG rendering.
- A truly "pure Rust" solution would mean either (a) rewriting the whole server in Rust (months of work, drops MathJax, the core value), or (b) adding a second Rust binary that pre-indexes labels/bib entries and exposes them via a sidecar protocol. Neither is cheap.
- The new features (ref/cite hover) are **parsing + cross-file resolution** — work that any language can do. They are independent of MathJax, so the Rust side could host them, but at the cost of splitting the LSP into two processes.

**Recommendation: extend the existing Node server.** It already has:
- The workspace walk (`preamble.ts`).
- The per-document text cache and re-parse hook.
- The single `onHover` handler — we just add a new branch in front of the math one.
- The render pipeline (math SVGs are exactly what we want for ref-target preview too).

If Rust work is desired later, the cleanest split is: keep Node for math (MathJax is the only credible option in 2026), move the new label/cite databases into a Rust binary that speaks a tiny JSON-RPC or stdio protocol with Node. That is a Phase-4 stretch goal; out of scope for the first implementation.

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

Markdown. No SVG. Author lists and titles can carry multi-line content, so we trim hard-wrapped lines (`trimMultiLineString`-style) and de-parenthesise (the brace-counting already handles nested braces, but we still want to strip one level of `{}` from field values — see Section 4.3).

### 3.3 What does **not** change

- LSP capabilities list stays `{ textDocumentSync: Full, hoverProvider: true }` — `hoverProvider` is a single boolean; one handler dispatches to whichever feature owns the cursor.
- The math hover path is untouched. We add a guard at the top of `hoverFor`: "is the cursor on a `\ref`/`\cite` key? if so, route to the new handler; otherwise, fall through to math."
- The render pipeline (MathJax) is reused for `\ref` to math.
- The configuration shape gains two new keys, both `true` by default; users can disable either independently.

## 4. Module breakdown

New files, all under `server/src/`:

| File                  | ~LoC | Responsibility                                                                 |
|-----------------------|-----:|--------------------------------------------------------------------------------|
| `labels.ts`           |  180 | Parse `\label{key}` definitions, `equation`/`thm`/… environment tracker, `\ref`/`\eqref`/… detector. |
| `bibtex.ts`           |  220 | Parse `.bib` files (entry types: `@article`, `@book`, `@inproceedings`, …). De-parenthesise fields, expand `@string` abbreviations. |
| `ref_hover.ts`        |   90 | Glue: given text+offset, decide if cursor is on a `\ref` key, look up the label, render (math or markdown). |
| `cite_hover.ts`       |   80 | Glue: given text+offset, decide if cursor is on a `\cite` key, look up the entry, format markdown. |
| `test/labels.test.ts` |  150 | Parser unit tests.                                                            |
| `test/bibtex.test.ts` |  150 | Parser unit tests, including nested braces, `@string`, malformed input.       |
| `test/ref_hover.test.ts` / `test/cite_hover.test.ts` |  120 | Integration: cursor at various positions returns the expected hover. |

Modified:

- `server/src/preamble.ts` — also walk `.bib` files; populate label/bib databases.
- `server/src/hover.ts` — dispatch table (math vs. ref vs. cite) at the top.
- `server/src/config.ts` — `enabledRefPreview: true`, `enabledCitePreview: true`, `bibMaxFileSizeMB: 5`.
- `server/src/server.ts` — keep the single `hoverProvider: true`; nothing else changes at the LSP-capability level.
- `server/src/cache.ts` — extend `LRU` reuse; per-text memoiser is generic.

Total new code: **~1 000 LoC** including tests.

## 5. Parsing strategy

### 5.1 Labels — `labels.ts`

`\label{key}` is recognised inside:
- Math environments we already track: `equation`, `equation*`, `align*?`, `gather*?`, `multline*?`.
- Theorems: `theorem`, `lemma`, `proposition`, `corollary`, `definition`, `remark`, `example`, `claim`, `conjecture`, plus their `*` forms. (Best-effort, no class introspection in v1.)
- Sections: `\section{…}\label{…}` even when not in a math env (handled as plain-text labels).

The scanner already walks the document left-to-right; **extend it to also emit label spans** as it goes. Specifically, when a math env closes, look at the contents of `\begin{…}` through the matching `\end{…}` for `\label{…}` — and also peek outside, into the trailing text up to the next paragraph, for labels attached to non-floats.

The label database is `Map<string, LabelEntry>` where:

```ts
interface LabelEntry {
  key: string;
  file: string;            // absolute path
  offset: number;          // byte offset of `\label{` opener
  line: number;            // LSP line of the `\label`
  env: EnvKind;            // "equation" | "theorem" | "section" | …
  math: { start: number; end: number } | null;  // math body range for SVG
  caption: string;         // best-effort human caption (theorem body, section title)
}
```

`\ref{key}` detection: at hover time, walk backward from cursor to the nearest `\ref|\eqref|\cref|\Cref|\autoref|\nameref|\pageref` whose opening brace encloses the cursor, and extract the key. Single-pass, brace-balanced, same `readBalancedBraces` helper we already have in `macros.ts` — **lift it to `server/src/text.ts`** so both modules share one definition.

### 5.2 BibTeX — `bibtex.ts`

Hand-rolled parser, scope-limited to what we need for hover previews:

- **Entry types** we care about: `@article`, `@book`, `@incollection`, `@inproceedings`, `@conference`, `@techreport`, `@phdthesis`, `@mastersthesis`, `@misc`, `@online`, `@unpublished`, plus `@string` (abbreviation) and `@comment` / `@preamble` / `@ignore` no-ops.
- **Field subset** shown in the hover: `author`, `title`, `journal`/`journaltitle`, `booktitle`, `publisher`, `year`, `volume`, `number`, `pages`, `editor`, `edition`, `series`, `address`, `doi`, `url`, `abstract`. Anything else is dropped from the preview but still parsed (so it is at least retained in the in-memory entry — useful for future jumps-to-definition).
- **Brace handling**: fields can be `{…}` (possibly multi-line, possibly nested), `"…"` (no nesting), or a concatenation of bare strings + `@string` macros. We parse all three forms. The brace counter from `macros.ts` is reused.
- **`@string` resolution**: collect them first in a pass, then substitute on the fly when building field values.
- **Error tolerance**: a malformed entry is dropped; the parser does not abort. We log the line number to the LSP `window/logMessage` channel so users can see why a particular entry is missing.

The bib database is `Map<string, BibEntry>` keyed on the citation key:

```ts
interface BibEntry {
  key: string;
  file: string;        // absolute .bib path
  offset: number;      // byte offset of `@article{` opener
  fields: Record<string, string>;
  type: BibType;
}
```

A **single .bib file can have thousands of entries**; we never copy it. We hold one map per file and look up by key with `O(1)` Map access. Total memory budget: ~1 KiB per entry × ~10 000 entries (a large Zotero library) = ~10 MiB — well within budget.

### 5.3 Cross-file `\input` / `\include`

The plan covers **directly included files** (`\input{...}`, `\include{...}`, `\subfileimport{...}`). Recursive resolution follows the same pattern as `preamble.ts`: each file is parsed once, results memoised by path, and edits trigger a single re-parse.

`\externaldocument` (the `xr` package) is **deferred** to Phase 3. The complexity is non-trivial (prefix remapping, conflict resolution) and only ~5% of users use it.

## 6. Hover dispatch — `hoverFor` flow

```
hoverFor(text, position, cfg):
  if !cfg.enabled: return null

  offset = positionToOffset(text, position)

  // NEW: try cite first, then ref, then math.  Order matters because
  // a \ref might be inside a math region (e.g. an equation body that
  // contains $\ref{foo}$), in which case the user almost certainly
  // means the ref, not the surrounding math.

  if cfg.enabledCitePreview:
    if r = citeHover(text, offset, ...): return r

  if cfg.enabledRefPreview:
    if r = refHover(text, offset, ...): return r

  // existing path
  return mathHover(text, offset, ...)
```

Each of the three sub-handlers returns `null` if the cursor isn't on its kind of target. The first non-null wins. The math path is unchanged.

The new handlers do **not** go through the `LRU` cache in the same way the math path does — math render is the slow step (MathJax, ~10-50 ms per formula), but `\ref` resolution is one Map lookup + maybe one render. We add a smaller, separate LRU of size 64 for ref-target renders only.

## 7. Workspace integration

Extend `preamble.ts` with two more maps, populated during the same walk that already happens in `initWorkspaceMacros` / `updateFileMacros`:

| Map                              | Source                                    | Populated by                          |
|----------------------------------|-------------------------------------------|---------------------------------------|
| `fileCache: Map<path, MacroMap>` *(existing)* | `.tex` files | `extractMacros`            |
| `labelCache: Map<labelKey, LabelEntry>` *(new)* | `.tex` files | `extractLabels`            |
| `bibCache: Map<bibKey, BibEntry>` *(new)*       | `.bib` files | `extractBibEntries`        |

The same root-inference / file-walk code is reused. `.bib` files are picked up by the same recursive scan that already finds `.tex` files — only the file-extension filter changes.

A single **file-type dispatcher** is the natural extraction: `parseFile(path, text) -> { macros, labels }` for `.tex` and `parseFile(path, text) -> { bibEntries }` for `.bib`. The current `updateFileMacros` and `initWorkspaceMacros` get a thin wrapper around this.

## 8. Edge cases & defensive behaviour

| Case                                        | Behaviour                                                                  |
|---------------------------------------------|----------------------------------------------------------------------------|
| Cursor on `\ref` outside any braces         | No preview. (Math path may still match if the surrounding text is `$…$`.)  |
| `\ref` to a non-existent label              | Hover shows `Reference 'foo' not found` (plain text), no crash.            |
| `\ref` to a section label (no math)         | Plain markdown with section title, no SVG.                                 |
| `.bib` file syntactically broken            | Drop the broken file, log a warning, keep other bib files.                 |
| Multiple `.bib` files with the same key     | First-loaded wins. Stable order via sorted file paths.                     |
| MathJax fails to render a ref target        | Fall back to the same `\`\`\`latex … \`\`\`` code block used by the math hover. |
| Document contains `\externaldocument{…}`    | Out of scope; ignore, do not crash.                                        |
| `\ref` inside a verbatim or comment         | Skipped at parse time — verbatim environments are already excluded by `scanner.ts`. |
| CRLF / `\r` in any of the new files         | Reuse the offset/position helpers from `scanner.ts`. Already CRLF-safe.    |

## 9. Rollout phases

Each phase is shippable and has its own tests.

### Phase 1 — label database + `\ref` hover (math targets only)

1. `labels.ts`: `extractLabels(text, file) -> LabelEntry[]`, focused on math environments only.
2. `ref_hover.ts`: cursor detection + lookup.
3. `preamble.ts` extension: also call `extractLabels` on every file parsed.
4. `hover.ts` dispatch.
5. `config.ts`: `enabledRefPreview: true`.
6. Tests: at least 12 unit tests in `labels.test.ts`, 6 integration tests in `ref_hover.test.ts`.

Acceptance: hovering on `\ref{...}` inside a document with a numbered equation shows the equation rendered as SVG plus a caption line.

### Phase 2 — theorem/lemma/etc. + non-math labels

1. Add theorem-like environments to a `NUMBERED_ENVS` table in `labels.ts`.
2. Best-effort caption extraction: for a theorem, take the first line of the body (everything between `\begin{thm}` and the next `\label` or paragraph break) and treat it as the caption.
3. For non-math labels (`\section`, `\subsection`, plain `\label{...}`), fall back to the section title or the text immediately around the label.

### Phase 3 — BibTeX + `\cite` hover

1. `bibtex.ts`: brace-counted parser, `@string` resolution, type-agnostic field capture.
2. `preamble.ts`: extend the file walk to pick up `.bib` files.
3. `cite_hover.ts`: cursor detection on `\cite`, `\citep`, `\citet`, `\citealp`, `\citeauthor`, `\citeyear` (and their `*` variants).
4. `config.ts`: `enabledCitePreview: true`, `bibMaxFileSizeMB: 5` (drop oversized bib files).
5. Tests: at least 15 in `bibtex.test.ts` covering nested braces, `@string`, concat fields, multi-line values, malformed input; 6 in `cite_hover.test.ts`.

### Phase 4 — polish & Rust stretch (optional)

- **In-file completion of `\cite` keys**: cheap extension once the bib database exists, but requires registering `completionProvider` in `server.ts`. May be added alongside Phase 3.
- **Caching invalidation granularity**: today the workspace macro cache invalidates per-file on `didChange`. Extend to labels and bibs; for `.bib` files, watch mtime.
- **`\externaldocument` (xr) support**: track external prefix per file, prepend on lookup.
- **Rust sidecar** (the user's hypothesis): a small `latex-index` binary that pre-parses every `.tex`/`.bib` and exposes a JSON-RPC `lookup(labelOrKey)` endpoint over stdio. Node would call it instead of doing the parse itself. This **only** makes sense if profiling shows parse-on-keystroke is the bottleneck — at 1-5 ms per file it almost certainly isn't. **Defer until measured.**

## 10. Open questions for the user

I am not asking, per the brief. Listed here so the next session can resolve them:

1. **Caching scope**: should the bib/label database persist across editor restarts? (Disk cache in OS temp dir, keyed by file path + mtime.) Or rebuild on launch? Rebuilding is ~10 ms for a typical project; persistence adds complexity for marginal benefit. **Default: rebuild.**
2. **Theorem caption source**: the `amsthm` body of a theorem is not standard. We can either (a) take the literal first line as caption (cheap, lossy), or (b) walk the macros to find a `\thmname{…}` or `\newtheorem{…}[…]` declaration (more correct, more code). **Default: (a) for v1.**
3. **`.bib` flavour**: BibLaTeX allows richer entry types and field names. **Default: support both, with a flag `bibtexBackend: 'bibtex' | 'biblatex'` defaulting to `biblatex`** (BibLaTeX is the dominant choice in 2026).

## 11. Risk register

| Risk                                                                | Mitigation                                                            |
|---------------------------------------------------------------------|-----------------------------------------------------------------------|
| Brace-counter miscounts inside a `\verb\|...\|`                    | Track `\verb` blocks the same way scanner.ts tracks verbatim envs.   |
| A `.bib` file changes externally (Zotero, Better BibTeX)            | Watch file mtime in `preamble.ts`; re-parse on change.               |
| MathJax takes 100+ ms to render a long ref target                   | Honour existing `maxFormulaLength` cap; fall back to code block.     |
| Two LSPs (this + official LaTeX) both walk the workspace            | Acceptable; we already do this for macros. Total walk <50 ms typical. |
| Phase 3 grows scope creep (full BibTeX parser)                      | Phased plan — Phase 3 has a strict field whitelist.                   |

## 12. Estimated effort

| Phase | Effort      | Why                                                       |
|-------|-------------|-----------------------------------------------------------|
| 1     | 1–2 days    | Reuse `scanner.ts` infrastructure; one new parser.        |
| 2     | 0.5 day     | Add an env table; caption heuristic.                      |
| 3     | 2–3 days    | Real parser; `@string` resolution; non-trivial testing.   |
| 4     | TBD         | Gated on profiling results and user feedback.             |
