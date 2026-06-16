# Legacy Node LSP

This directory contains the previous TypeScript/Node implementation of the
LaTeX Preview language server. It is retained temporarily for comparison tests
and migration reference.

The extension no longer launches this server in the normal runtime path. The
current language server is the Rust native `latex-preview-lsp` binary under
`latex-index/`.
