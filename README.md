# LaTeX Preview — Math hover for Zed

A companion extension for [Zed][zed] that renders LaTeX math formulas as SVG
images in hover tooltips.  Install it **alongside** the [official LaTeX
extension][zed-latex] (which provides completions, diagnostics, and build
tooling via texlab).

When you hover over a math formula, the extension expands any user-defined
macros found in your workspace and renders the result with MathJax.

## Requirements

- A bundled or user-configured **Rust native `latex-preview-lsp` binary**.
  During development, build it with `cd latex-index && cargo build --bin
  latex-preview-lsp`.

## Quick start

1. Install **both** extensions:
   - `LaTeX` (official — provides texlab for completions / diagnostics)
   - `LaTeX Preview (Math Hover)` (this one — provides hover SVGs)
2. Make sure `latex-preview-lsp` is bundled with the extension or available on
   your `PATH`.
3. Open a `.tex` file and hover over any math formula (`$E=mc^2$`,
   `\[ \int_\Omega \]`, …).
4. To adjust the size, add to your Zed `settings.json`:

```json
"lsp": {
  "latex-preview": {
    "settings": {
      "scale": 2.0
    }
  }
}
```

## Configuration

All settings live under `"lsp"."latex-preview"."settings"`:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | `boolean` | `true` | When `false`, no hover previews are shown |
| `scale` | `number` | `1.4` | SVG size multiplier (> 1 = larger) |
| `color` | `"auto"` \| `"black"` \| `"white"` | `"auto"` | SVG text colour. `auto` renders black — **dark-theme users should set `"white"`** |
| `timeoutMs` | `number` | `1500` | Maximum time (ms) spent rendering a formula |
| `maxFormulaLength` | `number` | `2000` | Skip formulas whose TeX source exceeds this |
| `enabledCitePreview` | `boolean` | `true` | When `false`, `\cite{…}` hovers are skipped |
| `enabledRefPreview` | `boolean` | `true` | When `false`, `\ref{…}`/`\eqref{…}` hovers are skipped |
| `enabledDocPreview` | `boolean` | `true` | When `false`, package/command doc hovers are skipped |

### Scale tuning

The `scale` setting controls the font size inside the rendered SVG:

- `1.0` — roughly matches the editor font size
- `1.4` — (default) slightly larger for readability
- `2.0` — noticeably larger, good for dense multi-line formulas

The hover popup has a fixed maximum width imposed by Zed.  For wide display
math (`$$…$$`, `\[…\]`) the image is scaled down to fit.  Increasing `scale`
makes the content bigger *before* that down-scaling, so more detail is
preserved.

## Supported math delimiters

| Delimiter | Mode |
|-----------|------|
| `$…$` | inline |
| `\(…\)` | inline |
| `$$…$$` | display |
| `\[…\]` | display |
| `\begin{equation}…\end{equation}` | display |
| `\begin{align}…\end{align}` | display |
| `\begin{gather}…\end{gather}` | display |
| `\begin{multline}…\end{multline}` | display |

Starred variants (`equation*`, `align*`, …) are also supported.

## Macro support (zero configuration)

User-defined macros are **automatically** discovered from every `.tex` file in
your workspace.  The scanner recognises:

- `\newcommand{\R}{\mathbb{R}}`
- `\newcommand*{\R}{\mathbb{R}}` (starred form)
- `\renewcommand{\R}{\mathbb{R}}`
- `\renewcommand*{\R}{\mathbb{R}}`
- `\providecommand{\R}{\mathbb{R}}`
- `\providecommand*{\R}{\mathbb{R}}`
- `\def\R{\mathbb{R}}`
- `\DeclareMathOperator{\div}{div}`

Macros defined in the *current document* take precedence over macros defined
in other workspace files.  Macros with arguments (`\newcommand{\norm}[1]{…}`)
are expanded, including multi-argument macros (`\newcommand{\foo}[2]{#1+#2}`
→ `\foo{a}{b}` becomes `a+b`).

## How it works

```
┌─ Zed ─────────────────────────────────────────────────┐
│  ┌─ latex (texlab)   ─── completions, diagnostics     │
│  └─ latex-preview ───── hover SVGs                    │
│       │                                                │
│       └─ latex-preview-lsp (Rust native LSP)           │
│            ├─ cursor.rs     cursor context / math scan │
│            ├─ macros.rs     extract & expand macros    │
│            ├─ labels.rs     ref target indexing        │
│            ├─ bibtex.rs     citation indexing          │
│            ├─ dict.rs       package / command docs     │
│            ├─ watcher.rs    workspace file updates     │
│            └─ lsp_main.rs   LSP + MathJax SVG hover    │
└────────────────────────────────────────────────────────┘
```

## Building from source

```bash
# Rust native LSP server
cd latex-index
cargo build --release --bin latex-preview-lsp

# Rust extension (WASI)
cd ..
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/latex_preview.wasm extension.wasm
```

Bundle the binary so the extension can find it without a user-set path:

```bash
mkdir -p bin
cp latex-index/target/release/latex-preview-lsp bin/   # .exe on Windows
```

Run the test suite:

```bash
cd latex-index
cargo test --bin latex-preview-lsp --test lsp_integration
```

## Language server lookup

The extension resolves the Rust LSP in this order:

1. `lsp.latex-preview.binary.path` in Zed settings
2. `latex-preview-lsp` on `PATH`
3. `<extension>/bin/latex-preview-lsp`
4. `<extension>/latex-index/target/release/latex-preview-lsp`
5. `<extension>/latex-index/target/debug/latex-preview-lsp`

On Windows, the binary name is `latex-preview-lsp.exe`.

For local development, a direct binary override is the most predictable setup:

```json
"lsp": {
  "latex-preview": {
    "binary": {
      "path": "/absolute/path/to/latex-index/target/release/latex-preview-lsp"
    }
  }
}
```

On Windows use the `.exe` suffix and forward slashes:
`C:/path/to/latex-index/target/release/latex-preview-lsp.exe`.

## Licence

MIT

[zed]: https://github.com/zed-industries/zed
[zed-latex]: https://github.com/rzukic/zed-latex
[repo]: https://github.com/pikel72/zed-latex-preview
