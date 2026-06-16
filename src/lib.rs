//! # LaTeX Preview — Math hover for Zed
//!
//! Companion extension for the official [LaTeX extension][tex].  Registers a
//! second language server (`latex-preview`) that renders LaTeX math formulas
//! as SVG tooltips on hover.
//!
//! The LSP itself is a Rust native binary (`latex-preview-lsp`) that handles
//! LSP transport, workspace indexing, hover dispatch, and MathJax-backed SVG
//! rendering.  This WASM stub only resolves the launch command and forwards the
//! user's `lsp.latex-preview.settings` to the server as LSP initialization
//! options.
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
//! | `enabledCitePreview` | `bool` | `true` | Toggle `\cite{…}` hover previews |
//! | `enabledRefPreview` | `bool` | `true` | Toggle `\ref{…}` hover previews |
//! | `enabledDocPreview` | `bool` | `true` | Toggle package/command doc hovers |

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
    /// 3. Bundled/development `latex-preview-lsp` binary in the extension tree
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

        // 3. Bundled/development Rust LSP binary.  Zed sets PWD to
        // `{extensions}/work/{ext_id}`; for dev extensions the real files live
        // at `{extensions}/installed/{ext_id}`, so mirror the old path rewrite.
        let dir = extension_dir();
        for candidate in lsp_binary_candidates(&dir) {
            if !candidate.is_empty() && std::fs::metadata(&candidate).is_ok() {
                return Ok(zed::Command {
                    command: candidate,
                    args: extra_args,
                    env: Default::default(),
                });
            }
        }

        // No candidate exists on disk.  Returning the conventional path would
        // hand Zed a path it cannot spawn, producing an opaque launch error.
        // Surface the missing-binary cause explicitly instead of silently
        // falling back to Node, which is no longer the runtime.
        Err("latex-preview: `latex-preview-lsp` was not found; build `latex-index` or set `lsp.latex-preview.binary.path`".into())
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

fn extension_dir() -> String {
    let pwd = std::env::var("PWD").unwrap_or_default();
    pwd.replace("\\work\\", "\\installed\\")
        .replace("/work/", "/installed/")
        .trim_end_matches(['/', '\\'])
        .to_string()
}

fn lsp_binary_candidates(dir: &str) -> Vec<String> {
    let exe = if matches!(zed::current_platform().0, zed::Os::Windows) {
        "latex-preview-lsp.exe"
    } else {
        "latex-preview-lsp"
    };
    vec![
        format!("{dir}/bin/{exe}"),
        format!("{dir}/latex-index/target/release/{exe}"),
        format!("{dir}/latex-index/target/debug/{exe}"),
    ]
}

zed::register_extension!(LatexPreviewExtension);
