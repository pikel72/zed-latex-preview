import { test } from "node:test";
import assert from "node:assert/strict";
import {
  citeHoverFor,
  cleanFieldValue,
  notFoundCiteHover,
} from "../src/cite_hover.js";
import type { BibEntry } from "../src/rpc_types.js";

function entry(overrides: Partial<BibEntry> & { fields: Record<string, string> }): BibEntry {
  return {
    key: "einstein1905",
    file: "/home/user/project/refs.bib",
    offset: 0,
    entry_type: "article",
    ...overrides,
  };
}

// ── cleanFieldValue ───────────────────────────────────────────────────

test("cleanFieldValue strips one level of outer braces", () => {
  assert.equal(cleanFieldValue("{Hello world}"), "Hello world");
});

test("cleanFieldValue collapses newline+space continuations", () => {
  assert.equal(cleanFieldValue("{Line one\n line two}"), "Line one line two");
});

test("cleanFieldValue collapses runs of internal whitespace", () => {
  assert.equal(cleanFieldValue("{a   b\t\tc}"), "a b c");
});

test("cleanFieldValue strips a leading brace when an inner brace closes first", () => {
  // stripOuterBraces walks to depth 0 at the `}` after `b`, and at that
  // point the position is the inner close (not the final char), so it
  // bails.  The actual function will then keep stripping the outer layer
  // because the post-trim string `{a{b}c}` is not symmetric the way the
  // helper wants.  In practice the result is `a{b}c` (outer braces go).
  // Document that.
  assert.equal(cleanFieldValue("{a{b}c}"), "a{b}c");
});

test("cleanFieldValue honours escaped braces", () => {
  // \{ should not bump the depth counter.
  assert.equal(cleanFieldValue("{\\{x\\}}"), "\\{x\\}");
});

test("cleanFieldValue leaves plain strings untouched", () => {
  assert.equal(cleanFieldValue("plain text"), "plain text");
});

test("cleanFieldValue trims surrounding whitespace", () => {
  assert.equal(cleanFieldValue("   {  hello  }   "), "hello");
});

// ── citeHoverFor: shape ───────────────────────────────────────────────

test("citeHoverFor returns a markdown hover with proper shape", () => {
  const result = citeHoverFor({
    found: true,
    entry: entry({ fields: { title: "On the Electrodynamics of Moving Bodies" } }),
  });
  assert.ok(result);
  assert.equal(result!.contents.kind, "markdown");
  assert.equal(typeof result!.contents.value, "string");
  assert.ok(result!.contents.value.includes("On the Electrodynamics of Moving Bodies"));
});

test("citeHoverFor omits range when none supplied", () => {
  const result = citeHoverFor({
    found: true,
    entry: entry({ fields: { title: "X" } }),
  });
  assert.equal(result!.range, undefined);
});

test("citeHoverFor includes a range when one is supplied", () => {
  const result = citeHoverFor(
    {
      found: true,
      entry: entry({ fields: { title: "X" } }),
    },
    [3, 11],
  );
  assert.ok(result!.range);
  assert.deepEqual(result!.range!.start, { line: 0, character: 3 });
  assert.deepEqual(result!.range!.end, { line: 0, character: 11 });
});

// ── citeHoverFor: full fields ─────────────────────────────────────────

test("citeHoverFor formats a full BibEntry: author/year heading, fields, path", () => {
  const e = entry({
    key: "lamport1986",
    file: "/abs/path/refs.bib",
    offset: 100,
    entry_type: "book",
    fields: {
      author: "Leslie Lamport",
      title: "LaTeX: A Document Preparation System",
      year: "1986",
      publisher: "Addison-Wesley",
    },
  });
  const r = citeHoverFor({ found: true, entry: e });
  assert.ok(r);
  const md = r!.contents.value;
  // Heading: "Author Year — *Title*"
  assert.match(md, /Leslie Lamport 1986 — \*LaTeX: A Document Preparation System\*/);
  // Author field omitted from body (already in heading).
  assert.ok(!md.includes("Authors: Leslie Lamport"), "Authors line should be omitted from heading");
  // Year omitted (already in heading).
  assert.ok(!md.includes("Year: 1986"), "Year line should be omitted from heading");
  // Publisher remains.
  assert.match(md, /^Publisher: Addison-Wesley/m);
  // File path header.
  assert.match(md, /File: .*refs\.bib:1/);
});

test("citeHoverFor includes all expected fields when present", () => {
  const e = entry({
    fields: {
      author: "Alan Turing",
      title: "Computing Machinery",
      year: "1950",
      journal: "Mind",
      booktitle: "Proc. of Something",
      volume: "59",
      number: "236",
      pages: "433--460",
      series: "New Series",
      edition: "2nd",
      address: "Oxford",
      doi: "10.1093/mind/lix.236.433",
      url: "https://example.com/turing",
      publisher: "OUP",
    },
  });
  const r = citeHoverFor({ found: true, entry: e });
  const md = r!.contents.value;
  assert.match(md, /Journal: Mind/);
  assert.match(md, /Booktitle: Proc\. of Something/);
  assert.match(md, /Volume: 59/);
  assert.match(md, /Number: 236/);
  assert.match(md, /Pages: 433--460/);
  assert.match(md, /Series: New Series/);
  assert.match(md, /Edition: 2nd/);
  assert.match(md, /Address: Oxford/);
  assert.match(md, /DOI: 10\.1093\/mind\/lix\.236\.433/);
  assert.match(md, /URL: https:\/\/example\.com\/turing/);
  assert.match(md, /Publisher: OUP/);
});

// ── citeHoverFor: missing fields ──────────────────────────────────────

test("citeHoverFor handles an entry with only a title", () => {
  const r = citeHoverFor({
    found: true,
    entry: entry({ fields: { title: "Untitled Work" } }),
  });
  const md = r!.contents.value;
  assert.match(md, /^\*Untitled Work\*$/m);
  // No Authors or Year lines.
  assert.ok(!md.includes("Authors:"));
  assert.ok(!md.includes("Year:"));
});

test("citeHoverFor handles an entry with author and year but no title", () => {
  const r = citeHoverFor({
    found: true,
    entry: entry({ fields: { author: "Some One", year: "1999" } }),
  });
  const md = r!.contents.value;
  // Heading is just the author.
  assert.match(md, /^Some One$/m);
  // Year line is still present (heading didn't include it).
  assert.match(md, /^Year: 1999$/m);
});

test("citeHoverFor handles a completely empty entry", () => {
  // Default entry() sets file to "/home/user/project/refs.bib", which has
  // 4 segments -> shortened to ".../project/refs.bib".  No heading or field
  // lines, so the output is a blank line followed by the path footer.
  const r = citeHoverFor({ found: true, entry: entry({ fields: {} }) });
  const md = r!.contents.value;
  assert.equal(md, "\nFile: .../project/refs.bib:1");
});

test("citeHoverFor drops fields whose cleaned value is empty", () => {
  const r = citeHoverFor({
    found: true,
    entry: entry({ fields: { title: "T", author: "   " } }),
  });
  const md = r!.contents.value;
  // Whitespace-only author should not produce a heading fragment.
  assert.ok(!md.includes("Authors:"));
  assert.ok(!md.includes("   "));
});

// ── citeHoverFor: not-found / abstract / brace unwrap ─────────────────

test("citeHoverFor returns a not-found message when entry is missing", () => {
  const r = citeHoverFor({ found: false });
  assert.ok(r);
  assert.equal(r!.contents.kind, "markdown");
  assert.match(r!.contents.value, /citation not found/);
});

test("notFoundCiteHover mirrors the not-found branch", () => {
  const a = notFoundCiteHover();
  assert.equal(a.contents.kind, "markdown");
  assert.match(a.contents.value, /citation not found/);
});

test("citeHoverFor unwraps outer braces from field values", () => {
  const r = citeHoverFor({
    found: true,
    entry: entry({ fields: { title: "{Braced Title}", publisher: "{Acme Press}" } }),
  });
  const md = r!.contents.value;
  assert.match(md, /Braced Title/);
  assert.ok(!md.includes("{Braced Title}"));
  assert.match(md, /Publisher: Acme Press/);
});

test("citeHoverFor collapses multi-line field values", () => {
  const r = citeHoverFor({
    found: true,
    entry: entry({
      fields: {
        title: "X",
        author: "Alice\n Smith and Bob\n Jones",
      },
    }),
  });
  const md = r!.contents.value;
  // Multi-line author value collapses to single line in the heading.
  assert.match(md, /Alice Smith and Bob Jones — \*X\*/);
});

test("citeHoverFor adds abstract as a block quote when present", () => {
  const r = citeHoverFor({
    found: true,
    entry: entry({
      fields: {
        title: "X",
        author: "Y",
        abstract: "{We prove a result about\n something interesting.}",
      },
    }),
  });
  const md = r!.contents.value;
  assert.match(md, /\n\n> We prove a result about something interesting\.$/m);
});

test("citeHoverFor shortens long file paths in the footer", () => {
  const e = entry({
    file: "/very/long/path/with/many/segments/refs.bib",
    fields: { title: "X" },
  });
  const r = citeHoverFor({ found: true, entry: e });
  const md = r!.contents.value;
  // Should keep only last 2 segments preceded by ".../"
  assert.match(md, /File: \.\.\.\/segments\/refs\.bib:1/);
});
