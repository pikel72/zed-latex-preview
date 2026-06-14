//! # LaTeX Preview — Math hover for Zed
//!
//! Companion extension for the official [LaTeX extension][tex].  Registers a
//! second language server (`latex-preview`) that renders LaTeX math formulas
//! as SVG tooltips on hover.
//!
//! The LSP itself is a bundled Node.js server (under `server/`) using MathJax
//! for TeX → SVG.  This Rust stub only resolves the launch command and forwards
//! the user's `lsp.latex-preview.settings` to the server as LSP
//! `initializationOptions`.
//!
//! ## Configuration
//!
//! All settings live under `"lsp"."latex-preview"."settings"` in Zed's
//! `settings.json`:
//!
//! | Key | Type | Default | Description |
//! |-----|------|---------|-------------|
//! | `enabled` | `bool` | `true` | Enable hover previews |
//! | `maxFormulaLength` | `usize` | `2000` | Max source length to render |
//! | `timeoutMs` | `u64` | `1500` | MathJax render timeout (ms) |
//! | `scale` | `f64` | `1.4` | SVG scale multiplier |
//! | `color` | `str` | `"auto"` | `"auto"`, `"black"`, or `"white"` |

use zed_extension_api::{self as zed};

#[derive(Default)]
struct LatexPreviewExtension;

/// Server name as registered in `extension.toml`.
const SERVER: &str = "latex-preview";

impl zed::Extension for LatexPreviewExtension {
    fn new() -> Self {
        Self::default()
    }

    /// Return the shell command that starts the `latex-preview` LSP.
    ///
    /// Resolution order:
    /// 1. User-provided `lsp.latex-preview.binary.path` setting
    /// 2. `latex-preview-lsp` binary on `PATH`
    /// 3. Bundled `server/out/src/server.js` run via Node
    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        if language_server_id.as_ref() != SERVER {
            return Err(format!("unknown language server: {language_server_id}"));
        }

        let lsp = zed::settings::LspSettings::for_worktree(SERVER, worktree).unwrap_or_default();
        let extra_args = match lsp.binary {
            Some(ref b) => b.arguments.clone().unwrap_or_default(),
            None => Vec::new(),
        };

        // 1. Explicit path from settings.
        if let Some(ref b) = lsp.binary {
            if let Some(ref path) = b.path {
                return Ok(zed::Command {
                    command: path.clone(),
                    args: extra_args,
                    env: Default::default(),
                });
            }
        }

        // 2. Standalone binary on PATH.
        if let Some(cmd) = worktree.which("latex-preview-lsp") {
            return Ok(zed::Command {
                command: cmd,
                args: extra_args,
                env: Default::default(),
            });
        }

        // 3. Bundled Node.js server.
        if let Some(node) = worktree.which("node") {
            // Zed sets PWD to `{extensions}/work/{ext_id}`, but for dev
            // extensions that directory is empty — the real files live at the
            // source, reachable via a symlink at
            // `{extensions}/installed/{ext_id}`.  Rewrite "work" → "installed"
            // so the resolved path follows the symlink to the real dir.
            // (WASI has no canonicalize(), so we rewrite the string by hand.)
            let pwd = std::env::var("PWD").unwrap_or_default();
            let dir = pwd
                .replace("\\work\\", "\\installed\\")
                .replace("/work/", "/installed/");
            let mut args = vec![
                format!("{}/server/out/src/server.js", dir.trim_end_matches('/')),
                "--stdio".to_string(),
            ];
            args.extend(extra_args);
            return Ok(zed::Command {
                command: node,
                args,
                env: Default::default(),
            });
        }

        Err("latex-preview: neither `latex-preview-lsp` nor `node` found on PATH".into())
    }

    /// Forward `lsp.latex-preview.settings` to the LSP as
    /// `initializationOptions`.  The server reads these via its
    /// `onInitialize` handler.
    fn language_server_initialization_options(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<Option<zed::serde_json::Value>> {
        if language_server_id.as_ref() != SERVER {
            return Ok(None);
        }
        let settings = zed::settings::LspSettings::for_worktree(SERVER, worktree)
            .ok()
            .and_then(|lsp| lsp.settings)
            .unwrap_or_default();
        Ok(Some(settings))
    }
}

zed::register_extension!(LatexPreviewExtension);
