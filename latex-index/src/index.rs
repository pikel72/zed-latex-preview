//! In-memory indexes for labels, citations, and macros.
//!
//! All three maps are `DashMap`s — `latex-index` handles one request at a
//! time in v1, but the DashMap shape lets Phase-2 add a real async runtime
//! without a rewrite.  See `docs/plan-ref-cite-hover.md` section 5.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A `\ref{...}` target (a `\label{...}` site in some file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelEntry {
    pub key: String,
    /// Absolute path of the file containing the `\label{...}`.
    pub file: PathBuf,
    /// Byte offset of the `\label{` opener.
    pub offset: usize,
    /// LSP line (0-based) of the `\label`.
    pub line: u32,
    /// Enclosing environment name: `equation`, `theorem`, `section`, …
    pub env: String,
    /// Byte range of the math body, when the label is inside a math env.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub math: Option<[usize; 2]>,
    /// Best-effort human caption (first line of theorem body, etc.).
    pub caption: String,
    /// Source-code snippet around the label, pre-formatted for hover.
    pub snippet: String,
}

/// A BibTeX entry (`@article`, `@book`, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibEntry {
    pub key: String,
    pub file: PathBuf,
    pub offset: usize,
    /// Field map in alphabetical order — keeps the JSON output stable.
    pub fields: BTreeMap<String, String>,
    /// Entry type name: `article`, `book`, `inproceedings`, …
    pub entry_type: String,
}

/// A user-defined macro.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroDef {
    pub name: String,
    /// File the macro was defined in (first definition wins on duplicates).
    pub file: PathBuf,
    pub body: String,
    pub arity: u32,
}

/// All three indexes held together.  Cheap to clone (DashMaps are Arc-backed).
#[derive(Debug, Default)]
pub struct Index {
    pub labels: DashMap<String, LabelEntry>,
    pub bib: DashMap<String, BibEntry>,
    pub macros: DashMap<String, MacroDef>,
}

impl Index {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything that came from `file`.  Called on `close_file`.
    pub fn remove_file(&self, file: &std::path::Path) {
        self.labels.retain(|_, v| v.file != file);
        self.bib.retain(|_, v| v.file != file);
        self.macros.retain(|_, v| v.file != file);
    }
}
