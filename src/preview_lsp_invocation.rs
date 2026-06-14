//! Resolves the command to start the `latex-preview-lsp` companion server.
//!
//! Resolution order:
//! 1. User-provided `lsp.latex-preview.binary.path` setting
//! 2. `latex-preview-lsp` on PATH (via `worktree.which`)
//! 3. Bundled `zed-latex/server/out/src/server.js` run via Node
use zed_extension_api as zed;

pub fn command(
    worktree: &zed::Worktree,
) -> Result<zed::Command, String> {
    use zed::settings::BinarySettings;
    let lsp_settings =
        zed::settings::LspSettings::for_worktree("latex-preview", worktree).unwrap_or_default();

    let args = match lsp_settings.binary {
        Some(BinarySettings { arguments: Some(ref a), .. }) => a.clone(),
        _ => vec![],
    };

    if let Some(BinarySettings { path: Some(ref p), .. }) = lsp_settings.binary {
        return Ok(zed::Command { command: p.clone(), args, env: Default::default() });
    }

    if let Some(cmd) = worktree.which("latex-preview-lsp") {
        return Ok(zed::Command { command: cmd, args, env: Default::default() });
    }

    if let Some(node) = worktree.which("node") {
        // Zed sets PWD to `{extensions}/work/{ext_id}`, but for dev extensions
        // that directory is empty — the real files stay at the source and are
        // reachable via a symlink at `{extensions}/installed/{ext_id}`.
        // For marketplace installs the files are extracted to both locations.
        // Replace "work" with "installed" so the resolved path follows the
        // symlink to the actual extension directory.
        // WASI does not support canonicalize(), so we join manually.
        let pwd = std::env::var("PWD").unwrap_or_default();
        let installed_dir = pwd.replace("\\work\\", "\\installed\\")
                               .replace("/work/", "/installed/");
        let server_js = format!("{}/server/out/src/server.js", installed_dir.trim_end_matches('/'));
        let mut full = vec![server_js, "--stdio".to_string()];
        full.extend(args);
        return Ok(zed::Command { command: node, args: full, env: Default::default() });
    }

    Err("latex-preview-lsp: neither `latex-preview-lsp` nor `node` found on PATH".into())
}
