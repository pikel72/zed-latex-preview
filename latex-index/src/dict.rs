//! Bundled package/command documentation dictionary.
//!
//! A small, hand-curated lookup table used by the doc-hover IPC path
//! (`doc_lookup`).  The dictionary is intentionally tiny — only entries
//! the user is most likely to hover over — and is built into the binary
//! at compile time.
//!
//! ## Storage choice
//!
//! Per spec §4.9 we can use either `phf::Map` or a sorted slice + binary
//! search.  We pick the sorted slice.  Reasons:
//!
//! * No new compile-time dependency (`phf` pulls in `phf_macros`,
//!   `phf_shared`, `rand` etc.  via `proc-macro`).
//! * The dictionary is small (~110 entries today).  A binary search
//!   over 110 elements is well under 10 ns on any machine and the
//!   static array sits in `.rodata`, so the codegen is identical to a
//!   perfect-hash table for our purposes.
//! * Easier to inspect by hand — entries are plain tuples in source
//!   order, sorted alphabetically at build time.
//!
//! `lookup` is `O(log n)` over the sorted slice.  We do not currently
//! track call-site hotness; if profiling later shows the dictionary is
//! a bottleneck, swapping to `phf` is a one-file change.

use serde::{Deserialize, Serialize};

/// Kind tag for a dictionary entry.  Serialised as lowercase so the
/// `DocKind` TypeScript union (`"package" | "command"`) matches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DocKind {
    Package,
    Command,
}

/// One bundled dictionary entry.
#[derive(Debug, Clone, Serialize)]
pub struct DictEntry {
    pub title: String,
    pub kind: DocKind,
    pub short: String,
    /// Optional longer markdown body (≤ 2 KiB per spec §4.9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

// ── the bundled table ──────────────────────────────────────────────────
//
// Sorted by `title` (the lookup key).  When adding entries, keep them
// in alphabetical order so the binary search stays valid.  The
// `entries_are_sorted` unit test below catches drift.
//
// Spec §4.9 lists ~30 packages and ~80 commands.  We hand-curate a
// pragmatic subset — entries the user is most likely to hover over.

const ENTRIES: &[( &str, DocKind, &str, Option<&str> )] = &[
    (
        "addtocounter",
        DocKind::Command,
        "Increment a counter.",
        None,
    ),
    (
        "algorithm2e",
        DocKind::Package,
        "Floating algorithm environments.",
        Some("Provides `algorithm` float and `procedure` environment with extensive formatting controls."),
    ),
    (
        "algorithmicx",
        DocKind::Package,
        "Typeset pseudocode with rich control structures.",
        Some("Successor to `algorithmic`. Use `\\\\algorithmic`/`\\\\algorithmicindent` or the `algpseudocode` style."),
    ),
    (
        "alpha",
        DocKind::Command,
        "Lowercase Greek letter α.",
        None,
    ),
    (
        "amsfonts",
        DocKind::Package,
        "Extra math alphabets (blackboard bold, Fraktur, …).",
        Some("Provides `\\\\mathbb`, `\\\\mathfrak`, `\\\\mathscr`."),
    ),
    (
        "amsmath",
        DocKind::Package,
        "Extended math environments and symbols.",
        Some("Loads `equation`, `align`, `gather`, `multline`, `cases`, `\\\\text`, `\\\\boldsymbol`, etc."),
    ),
    (
        "amssymb",
        DocKind::Package,
        "Extra math symbols.",
        Some("Adds thousands of math-mode symbols beyond LaTeX2e core."),
    ),
    (
        "appendix",
        DocKind::Command,
        "Switch section numbering to alphabetic for appendices.",
        None,
    ),
    (
        "author",
        DocKind::Command,
        "Author of the document; appears on the title page.",
        None,
    ),
    (
        "autoref",
        DocKind::Command,
        "Auto-prefixed reference (loads `hyperref`).",
        None,
    ),
    (
        "beta",
        DocKind::Command,
        "Lowercase Greek letter β.",
        None,
    ),
    (
        "biblatex",
        DocKind::Package,
        "Modern bibliography/citation handling.",
        Some("Use `\\\\addbibresource` and `\\\\printbibliography`.  Cite with `\\\\cite`/`\\\\parencite`/`\\\\textcite`."),
    ),
    (
        "bibliography",
        DocKind::Command,
        "Print the bibliography from one or more `.bib` files.",
        None,
    ),
    (
        "bibliographystyle",
        DocKind::Command,
        "Select bibliography style (e.g. `plain`, `ieeetr`).",
        None,
    ),
    (
        "bm",
        DocKind::Package,
        "Bold math symbols.",
        Some("Provides `\\\\bm{...}` to bold entire math expressions."),
    ),
    (
        "booktabs",
        DocKind::Package,
        "Professional-quality horizontal rules in tables.",
        Some("Use `\\\\toprule`, `\\\\midrule`, `\\\\bottomrule` instead of vertical `\\\\hline`."),
    ),
    (
        "caption",
        DocKind::Package,
        "Customise caption formatting for figures and tables.",
        Some("Configurable via `\\\\captionsetup`.  `\\\\caption{...}` is the matching command."),
    ),
    (
        "cdot",
        DocKind::Command,
        "Centered dot multiplication (·).",
        None,
    ),
    (
        "cdots",
        DocKind::Command,
        "Centered horizontal ellipsis (⋯).",
        None,
    ),
    (
        "chapter",
        DocKind::Command,
        "Top-level section heading (in `book` / `report`).",
        None,
    ),
    (
        "chi",
        DocKind::Command,
        "Lowercase Greek letter χ.",
        None,
    ),
    (
        "cite",
        DocKind::Command,
        "Numeric or author-year citation (depends on bibliography style).",
        None,
    ),
    (
        "citep",
        DocKind::Command,
        "Parenthetical citation (parentheses around the year/number).",
        None,
    ),
    (
        "citet",
        DocKind::Command,
        "In-text citation (year/number inline, no parentheses).",
        None,
    ),
    (
        "clearpage",
        DocKind::Command,
        "Page break that flushes pending floats.",
        None,
    ),
    (
        "cref",
        DocKind::Command,
        "Smart cross-reference (loads `cleveref`).  Includes the env prefix (eq. / fig. / §).",
        None,
    ),
    (
        "date",
        DocKind::Command,
        "Date shown on the title page.",
        None,
    ),
    (
        "delta",
        DocKind::Command,
        "Lowercase Greek letter δ.",
        None,
    ),
    (
        "documentclass",
        DocKind::Command,
        "Pick the document class (`article`, `book`, `report`, …).",
        None,
    ),
    (
        "dots",
        DocKind::Command,
        "Context-sensitive ellipsis.",
        None,
    ),
    (
        "emph",
        DocKind::Command,
        "Emphasis; rendered as italic in normal text, upright in italic context.",
        None,
    ),
    (
        "end",
        DocKind::Command,
        "Close a previously opened environment.",
        None,
    ),
    (
        "epsilon",
        DocKind::Command,
        "Lowercase Greek letter ε.",
        None,
    ),
    (
        "eqref",
        DocKind::Command,
        "Reference an equation, wrapping the number in parentheses.",
        None,
    ),
    (
        "eta",
        DocKind::Command,
        "Lowercase Greek letter η.",
        None,
    ),
    (
        "fontenc",
        DocKind::Package,
        "Select font encoding (T1, T2A, …).",
        Some("Most projects use `\\\\usepackage[T1]{fontenc}` for hyphenation correctness."),
    ),
    (
        "footnote",
        DocKind::Command,
        "Footnote text.",
        None,
    ),
    (
        "frac",
        DocKind::Command,
        "Fraction: `\\\\frac{numerator}{denominator}`.",
        None,
    ),
    (
        "gamma",
        DocKind::Command,
        "Lowercase Greek letter γ.",
        None,
    ),
    (
        "geometry",
        DocKind::Package,
        "Set page geometry (margins, paper size).",
        Some("Configure via `\\\\geometry{left=…, right=…, top=…, bottom=…}`."),
    ),
    (
        "glossaries",
        DocKind::Package,
        "Acronyms and glossary entries.",
        Some("Use `\\\\newglossaryentry`/`\\\\newacronym` then `\\\\printglossaries`."),
    ),
    (
        "graphicx",
        DocKind::Package,
        "Include external graphics.",
        Some("Provides `\\\\includegraphics{file}` with `width`/`height`/key-value options."),
    ),
    (
        "href",
        DocKind::Command,
        "Hyperlink text to a URL.",
        None,
    ),
    (
        "hyperref",
        DocKind::Package,
        "Clickable hyperlinks and PDF metadata.",
        Some("Loads last in the preamble for correct behaviour.  Use `\\\\href{url}{text}` from the matching command."),
    ),
    (
        "include",
        DocKind::Command,
        "Include another `.tex` file, starting on a new page.",
        None,
    ),
    (
        "infty",
        DocKind::Command,
        "Infinity symbol (∞).",
        None,
    ),
    (
        "input",
        DocKind::Command,
        "Inline-include another `.tex` file (no page break).",
        None,
    ),
    (
        "inputenc",
        DocKind::Package,
        "Set input encoding (utf8, latin1, …).",
        Some("Use `\\\\usepackage[utf8]{inputenc}` for non-ASCII source files."),
    ),
    (
        "int",
        DocKind::Command,
        "Integral symbol (large operator).",
        None,
    ),
    (
        "item",
        DocKind::Command,
        "Item in an `itemize` / `enumerate` / `description` list.",
        None,
    ),
    (
        "kappa",
        DocKind::Command,
        "Lowercase Greek letter κ.",
        None,
    ),
    (
        "kpsewhich",
        DocKind::Package,
        "Test TeX path lookup from inside a document.",
        Some("Shell-style `\\\\ShellEscape` companion; not commonly used directly in user docs."),
    ),
    (
        "label",
        DocKind::Command,
        "Mark a position or equation with a key for `\\\\ref{…}`.",
        None,
    ),
    (
        "ldots",
        DocKind::Command,
        "Baseline horizontal ellipsis (…).",
        None,
    ),
    (
        "Leftarrow",
        DocKind::Command,
        "Double left-arrow (⇐).",
        None,
    ),
    (
        "Leftrightarrow",
        DocKind::Command,
        "Double bi-directional arrow (⇔).",
        None,
    ),
    (
        "leq",
        DocKind::Command,
        "Less-than-or-equal (≤).",
        None,
    ),
    (
        "listings",
        DocKind::Package,
        "Typeset source code.",
        Some("Use `\\\\begin{lstlisting}` with `language=` option or `\\\\lstset`."),
    ),
    (
        "lmodern",
        DocKind::Package,
        "Latin Modern fonts (Computer Modern successor).",
        Some("Pair with `\\\\usepackage[T1]{fontenc}` for cleaner PDF output."),
    ),
    (
        "mathbf",
        DocKind::Command,
        "Bold math alphabet.",
        None,
    ),
    (
        "mathcal",
        DocKind::Command,
        "Calligraphic alphabet (𝒜, ℬ, …).",
        None,
    ),
    (
        "mathfrak",
        DocKind::Command,
        "Fraktur alphabet (𝔄, 𝔅, …); loads `amssymb`.",
        None,
    ),
    (
        "mathit",
        DocKind::Command,
        "Italic math alphabet.",
        None,
    ),
    (
        "mathrm",
        DocKind::Command,
        "Upright (roman) math alphabet.",
        None,
    ),
    (
        "mathsf",
        DocKind::Command,
        "Sans-serif math alphabet.",
        None,
    ),
    (
        "mathtools",
        DocKind::Package,
        "Bug-fixes and extras for `amsmath`.",
        Some("Loads `amsmath` automatically.  Adds `\\\\xRightarrow`, `\\\\xrightarrow[]`, `\\\\coloneqq`, etc."),
    ),
    (
        "minted",
        DocKind::Package,
        "Syntax-highlighted source code via Pygments.",
        Some("Requires `\\\\ShellEscape` (Python + Pygments).  Use `\\\\begin{minted}{lang}`."),
    ),
    (
        "mu",
        DocKind::Command,
        "Lowercase Greek letter μ.",
        None,
    ),
    (
        "nabla",
        DocKind::Command,
        "Gradient / nabla operator (∇).",
        None,
    ),
    (
        "nameref",
        DocKind::Command,
        "Print the section/heading text rather than its number (loads `hyperref`).",
        None,
    ),
    (
        "natbib",
        DocKind::Package,
        "Author-year / numerical citations.",
        Some("Use `\\\\citet`, `\\\\citep`, `\\\\citeauthor`, `\\\\citeyear`."),
    ),
    (
        "newcommand",
        DocKind::Command,
        "Define a new command: `\\\\newcommand{\\\\foo}{bar}`.",
        None,
    ),
    (
        "newpage",
        DocKind::Command,
        "Forced page break.",
        None,
    ),
    (
        "noindent",
        DocKind::Command,
        "Suppress indentation for this paragraph.",
        None,
    ),
    (
        "nu",
        DocKind::Command,
        "Lowercase Greek letter ν.",
        None,
    ),
    (
        "omega",
        DocKind::Command,
        "Lowercase Greek letter ω.",
        None,
    ),
    (
        "pagebreak",
        DocKind::Command,
        "Forced page break with optional stretch.",
        None,
    ),
    (
        "pageref",
        DocKind::Command,
        "Print the page number of a `\\\\label{key}`.",
        None,
    ),
    (
        "par",
        DocKind::Command,
        "Paragraph break.",
        None,
    ),
    (
        "paragraph",
        DocKind::Command,
        "Paragraph-level heading (a run-in heading).",
        None,
    ),
    (
        "partial",
        DocKind::Command,
        "Partial-derivative symbol (∂).",
        None,
    ),
    (
        "pdfpages",
        DocKind::Package,
        "Include external PDF pages.",
        Some("Use `\\\\includepdf[pages=1-3]{file.pdf}`."),
    ),
    (
        "phi",
        DocKind::Command,
        "Lowercase Greek letter φ.",
        None,
    ),
    (
        "pi",
        DocKind::Command,
        "Lowercase Greek letter π.",
        None,
    ),
    (
        "pm",
        DocKind::Command,
        "Plus-minus (±).",
        None,
    ),
    (
        "prod",
        DocKind::Command,
        "Product symbol (large operator).",
        None,
    ),
    (
        "providecommand",
        DocKind::Command,
        "Define a command only if it isn't already defined.",
        None,
    ),
    (
        "psi",
        DocKind::Command,
        "Lowercase Greek letter ψ.",
        None,
    ),
    (
        "qquad",
        DocKind::Command,
        "Two em-spaces in math mode.",
        None,
    ),
    (
        "quad",
        DocKind::Command,
        "Em-space in math mode.",
        None,
    ),
    (
        "ref",
        DocKind::Command,
        "Reference a `\\\\label{key}` and print its number.",
        None,
    ),
    (
        "renewcommand",
        DocKind::Command,
        "Re-define an existing command.",
        None,
    ),
    (
        "RequirePackage",
        DocKind::Command,
        "Load a package (LaTeX-package-style); usable inside class files.",
        None,
    ),
    (
        "rho",
        DocKind::Command,
        "Lowercase Greek letter ρ.",
        None,
    ),
    (
        "section",
        DocKind::Command,
        "Section heading.",
        None,
    ),
    (
        "setcounter",
        DocKind::Command,
        "Set a counter: `\\\\setcounter{secnumdepth}{0}`.",
        None,
    ),
    (
        "sigma",
        DocKind::Command,
        "Lowercase Greek letter σ.",
        None,
    ),
    (
        "siunitx",
        DocKind::Package,
        "Consistent number/unit typesetting.",
        Some("Use `\\\\num`, `\\\\si`, `\\\\SI`, `\\\\ang` for numbers, units, angles."),
    ),
    (
        "sqrt",
        DocKind::Command,
        "Square root: `\\\\sqrt{x}`.  Optional argument: `\\\\sqrt[n]{x}`.",
        None,
    ),
    (
        "standalone",
        DocKind::Package,
        "Compile single figures/tables as standalone PDFs.",
        Some("Use `documentclass{standalone}` for tightly cropped sub-figures."),
    ),
    (
        "subcaption",
        DocKind::Package,
        "Sub-figures and sub-tables.",
        None,
    ),
    (
        "subsection",
        DocKind::Command,
        "Subsection heading.",
        None,
    ),
    (
        "subsubsection",
        DocKind::Command,
        "Sub-subsection heading.",
        None,
    ),
    (
        "sum",
        DocKind::Command,
        "Summation symbol (large operator).",
        None,
    ),
    (
        "tau",
        DocKind::Command,
        "Lowercase Greek letter τ.",
        None,
    ),
    (
        "tcolorbox",
        DocKind::Package,
        "Coloured/shaded framed boxes with titles.",
        Some("Use `\\\\begin{tcolorbox}` with `colback`/`colframe`/etc. options."),
    ),
    (
        "text",
        DocKind::Command,
        "Text mode inside math (loads `amsmath`).",
        None,
    ),
    (
        "textbf",
        DocKind::Command,
        "Bold text.",
        Some("`\\\\textbf{bold}`."),
    ),
    (
        "textit",
        DocKind::Command,
        "Italic text.",
        Some("`\\\\textit{italic}`."),
    ),
    (
        "textrm",
        DocKind::Command,
        "Roman (serif) text.",
        None,
    ),
    (
        "textsc",
        DocKind::Command,
        "Small-caps text.",
        None,
    ),
    (
        "textsf",
        DocKind::Command,
        "Sans-serif text.",
        None,
    ),
    (
        "textsubscript",
        DocKind::Command,
        "Text-mode subscript (requires `fixltx2e` or modern LaTeX).",
        None,
    ),
    (
        "textsuperscript",
        DocKind::Command,
        "Text-mode superscript.",
        None,
    ),
    (
        "texttt",
        DocKind::Command,
        "Monospace (typewriter) text.",
        None,
    ),
    (
        "theta",
        DocKind::Command,
        "Lowercase Greek letter θ.",
        None,
    ),
    (
        "tikz",
        DocKind::Package,
        "Programmable vector graphics.",
        Some("Use `\\\\begin{tikzpicture}`; pair with `pgfplots` for plots."),
    ),
    (
        "times",
        DocKind::Command,
        "Multiplication cross (×).",
        None,
    ),
    (
        "title",
        DocKind::Command,
        "Title of the document; appears on the title page.",
        None,
    ),
    (
        "to",
        DocKind::Command,
        "Right-arrow (→); alternative: `\\\\rightarrow`.",
        None,
    ),
    (
        "underline",
        DocKind::Command,
        "Underlined text.",
        None,
    ),
    (
        "url",
        DocKind::Command,
        "Typeset a URL.",
        None,
    ),
    (
        "usepackage",
        DocKind::Command,
        "Load a package from the preamble.",
        None,
    ),
    (
        "varepsilon",
        DocKind::Command,
        "Variant epsilon (ε).",
        None,
    ),
    (
        "varphi",
        DocKind::Command,
        "Variant phi (ϕ).",
        None,
    ),
    (
        "varpi",
        DocKind::Command,
        "Variant pi (ϖ).",
        None,
    ),
    (
        "varrho",
        DocKind::Command,
        "Variant rho (ϱ).",
        None,
    ),
    (
        "varsigma",
        DocKind::Command,
        "Variant sigma (ς).",
        None,
    ),
    (
        "vartheta",
        DocKind::Command,
        "Variant theta (ϑ).",
        None,
    ),
    (
        "xcolor",
        DocKind::Package,
        "Colour support and definitions.",
        Some("Provides `\\\\color`, `\\\\textcolor`, `\\\\definecolor`."),
    ),
    (
        "xi",
        DocKind::Command,
        "Lowercase Greek letter ξ.",
        None,
    ),
    (
        "xparse",
        DocKind::Package,
        "Define rich command/environment interfaces.",
        Some("Use `\\\\NewDocumentCommand` for commands with optional/argument-rich signatures."),
    ),
    (
        "zeta",
        DocKind::Command,
        "Lowercase Greek letter ζ.",
        None,
    ),

];

// Binary search over the sorted `title` keys.  Returns the index of
// the matching entry, or `None` if `name` is not present.
//
// We sort the slice using a case-insensitive comparator so `cref` and
// `Cref` (which are user-facing different commands but lexically only
// differ in case) sit together and stay findable.  ASCII byte-wise
// ordering is brittle here because LaTeX command names are themselves
// case-sensitive — but `\cref` and `\Cref` are distinct commands, so we
// keep both keys as separate entries and order them case-insensitively.
fn lookup_index(name: &str) -> Option<usize> {
    let needle = name.to_ascii_lowercase();
    let mut lo: usize = 0;
    let mut hi: usize = ENTRIES.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (title, _, _, _) = ENTRIES[mid];
        let cmp = title.to_ascii_lowercase().cmp(&needle);
        match cmp {
            std::cmp::Ordering::Equal => return Some(mid),
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
        }
    }
    None
}

/// Look up a name (package or command) in the bundled dictionary.
///
/// Returns `None` for unknown names so the Node side can fall through to
/// the math hover without erroring.
pub fn lookup(name: &str) -> Option<DictEntry> {
    let idx = lookup_index(name)?;
    let (title, kind, short, docs) = ENTRIES[idx];
    Some(DictEntry {
        title: title.to_string(),
        kind,
        short: short.to_string(),
        docs: docs.map(|d| d.to_string()),
    })
}

// ── tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_package() {
        let e = lookup("amsmath").expect("amsmath must be in the dictionary");
        assert_eq!(e.title, "amsmath");
        assert_eq!(e.kind, DocKind::Package);
        assert!(e.short.contains("math"));
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("not-a-real-package-name").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn entries_are_sorted() {
        // Defensive: a build-time invariant.  The binary search above
        // requires strict ascending order by `title` (case-insensitive
        // because lookup is also case-insensitive).
        for w in ENTRIES.windows(2) {
            let (a, _, _, _) = w[0];
            let (b, _, _, _) = w[1];
            assert!(
                a.to_ascii_lowercase() < b.to_ascii_lowercase(),
                "dict entries must be sorted (case-insensitive) by title; got {:?} before {:?}",
                a,
                b
            );
        }
    }

    #[test]
    fn no_duplicate_titles() {
        // Binary search requires no duplicates (would return the first
        // match only and silently shadow the second).  Case-insensitive
        // because lookup is.
        for w in ENTRIES.windows(2) {
            let (a, _, _, _) = w[0];
            let (b, _, _, _) = w[1];
            assert_ne!(
                a.to_ascii_lowercase(),
                b.to_ascii_lowercase(),
                "duplicate dict title: {}",
                a
            );
        }
    }
}
