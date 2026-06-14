# LaTeX Preview — Math hover for Zed

A companion extension for [Zed][zed] that renders LaTeX math formulas as SVG
images in hover tooltips.  Install it **alongside** the [official LaTeX
extension][zed-latex] (which provides completions, diagnostics, and build
tooling via texlab).

When you hover over a math formula, the extension expands any user-defined
macros found in your workspace and renders the result with MathJax.

## Requirements

- **Node.js 18+** must be on your `PATH` (the LSP server is a bundled Node.js
  program that uses MathJax for rendering).

## Quick start

1. Install **both** extensions:
   - `LaTeX` (official — provides texlab for completions / diagnostics)
   - `LaTeX Preview (Math Hover)` (this one — provides hover SVGs)
2. Make sure Node.js 18+ is on your `PATH`.
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
| `color` | `"auto"` \| `"black"` \| `"white"` | `"auto"` | SVG text colour |
| `timeoutMs` | `number` | `1500` | Maximum time (ms) spent rendering a formula |
| `maxFormulaLength` | `number` | `2000` | Skip formulas whose TeX source exceeds this |

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
│       └─ node server/out/src/server.js                 │
│            ├─ scanner.ts    find math region           │
│            ├─ macros.ts     extract & expand macros    │
│            ├─ preamble.ts   scan workspace .tex files  │
│            ├─ render.ts     MathJax TeX → SVG          │
│            ├─ cache.ts      LRU render cache           │
│            ├─ config.ts     user settings              │
│            └─ server.ts     LSP protocol glue          │
└────────────────────────────────────────────────────────┘
```

## Building from source

```bash
# TypeScript LSP server
cd server
npm ci
npx tsc -p tsconfig.json

# Rust extension (WASI)
cargo build --target wasm32-wasip2
cp target/wasm32-wasip2/debug/latex_preview.wasm extension.wasm
```

Run the test suite:

```bash
cd server
npx tsx --test test/scanner.test.ts
npx tsx --test test/macros.test.ts
npx tsx --test test/render.smoke.ts
npx tsx --test test/hover.test.ts
npx tsx --test test/cache.test.ts
```

## Licence

MIT

[zed]: https://github.com/zed-industries/zed
[zed-latex]: https://github.com/rzukic/zed-latex
[repo]: https://github.com/pikel72/zed-latex-preview
