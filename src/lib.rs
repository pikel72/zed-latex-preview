//! # LaTeX Preview — Math hover for Zed
//!
//! A companion extension for the official [LaTeX extension][tex].
//! It registers a second language server (`latex-preview`) that renders LaTeX
//! math formulas as SVG tooltips on hover.
//!
//! The LSP is a bundled Node.js server (under `server/`) that uses MathJax
//! for TeX → SVG rendering.  The Rust side only handles LSP discovery and
//! hands off workspace configuration to the server.
//!
//! ## Architecture
//!
//! ```text
//! Zed                                   Rust stub (this crate)
//!  │                                          │
//!  ├─ latex (texlab) ─── completions/diag     │  official extension
//!  │                                          │
//!  └─ latex-preview ─── hover tooltips        │  this extension
//!       │                                      │
//!       └─ Node.js LSP  ─── MathJax ── SVG     │  server/out/src/server.js
//! ```
//!
//! [tex]: https://github.com/rzukic/zed-latex
//!
//! ## Configuration
//!
//! All settings live under `"lsp"."latex-preview"` in Zed's `settings.json`:
//!
//! | Key | Type | Default | Description |
//! |-----|------|---------|-------------|
//! | `enabled` | `bool` | `true` | Enable hover previews |
//! | `maxFormulaLength` | `usize` | `2000` | Max source length to render |
//! | `timeoutMs` | `u64` | `1500` | MathJax render timeout (ms) |
//! | `scale` | `f64` | `1.4` | SVG scale multiplier |
//! | `color` | `str` | `"auto"` | `"auto"`, `"black"`, or `"white"` |

mod preview_lsp_invocation;
mod preview_lsp_workspace_config;

use zed_extension_api::{self as zed};

#[derive(Default)]
struct LatexPreviewExtension;

impl zed::Extension for LatexPreviewExtension {
    fn new() -> Self {
        Self::default()
    }

    /// Return the shell command that starts the `latex-preview` LSP.
    ///
    /// Resolution order:
    /// 1. User-provided `lsp.latex-preview.binary.path` setting
    /// 2. `latex-preview-lsp` binary on `PATH`
    /// 3. Bundled `server/out/src/server.js` run via Node.js
    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        match language_server_id.as_ref() {
            "latex-preview" => preview_lsp_invocation::command(worktree),
            id => Err(format!("unknown language server: {id}")),
        }
    }

    /// Forward user settings (`lsp.latex-preview.settings`) to the LSP as
    /// workspace configuration.
    fn language_server_workspace_configuration(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        match language_server_id.as_ref() {
            "latex-preview" => {
                let settings = zed::settings::LspSettings::for_worktree(
                    "latex-preview",
                    worktree,
                )
                .ok()
                .and_then(|lsp| lsp.settings.clone())
                .unwrap_or_default();
                Ok(preview_lsp_workspace_config::get(settings))
            }
            _ => Ok(None),
        }
    }
}

zed::register_extension!(LatexPreviewExtension);
