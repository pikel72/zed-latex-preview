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

        // 3. Bundled/development Rust LSP binary.  Try two PWD variants:
        //    - raw PWD (Zed's working dir for the extension)
        //    - PWD with `work/` rewritten to `installed/` (Zed's dev
        //      install convention where the install dir is a junction
        //      into the source tree)
        //    For each, probe `bin/`, `latex-index/target/release/`, and
        //    `latex-index/target/debug/`.
        let exe = lsp_binary_name();
        let pwd_raw = std::env::var("PWD").unwrap_or_default();
        let pwd_raw_trim = pwd_raw.trim_end_matches(['/', '\\']).to_string();
        let pwd_installed = pwd_raw
            .replace("\\work\\", "\\installed\\")
            .replace("/work/", "/installed/")
            .trim_end_matches(['/', '\\'])
            .to_string();
        let mut tried = Vec::new();
        for dir in [pwd_raw_trim, pwd_installed] {
            if dir.is_empty() {
                continue;
            }
            for sub in ["bin", "latex-index/target/release", "latex-index/target/debug"] {
                let candidate = format!("{dir}/{sub}/{exe}");
                if std::fs::metadata(&candidate).is_ok() {
                    return Ok(zed::Command {
                        command: candidate,
                        args: extra_args,
                        env: Default::default(),
                    });
                }
                tried.push(candidate);
            }
        }

        // No candidate exists on disk.  Tell the user exactly where we
        // looked and what to do, rather than handing Zed a path it can't
        // spawn.  Falling back to Node is no longer an option.
        let tried_list = tried.join("\n      ");
        Err(format!(
            "latex-preview: `{exe}` was not found.  Looked in:\n      {tried_list}\n  \
             Fix one of:\n    \
             1. Put `{exe}` on your PATH and reload Zed.\n    \
             2. Set `lsp.latex-preview.binary.path` in Zed settings to an absolute path.\n    \
             3. Build it and copy it into `bin/` next to `extension.wasm`:\n         \
                cd latex-index && cargo build --release --bin latex-preview-lsp\n         \
                cp target/release/{exe} ../bin/"
        )
        .into())
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

fn lsp_binary_name() -> &'static str {
    if matches!(zed::current_platform().0, zed::Os::Windows) {
        "latex-preview-lsp.exe"
    } else {
        "latex-preview-lsp"
    }
}

zed::register_extension!(LatexPreviewExtension);
