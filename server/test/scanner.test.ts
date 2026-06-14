import { test } from "node:test";
import assert from "node:assert/strict";
import { findMathAt } from "../src/scanner.js";

// Each case: [src, offset, wantSource, wantDisplay, wantStart, wantEnd]
// `wantStart`/`wantEnd` are the full delimiter-inclusive range.
const cases: Array<[string, number, string, boolean, number, number]> = [
  //                              source        display start end(off)
  ["let $E = mc^2$.",            4, "E = mc^2", false, 4, 14],
  ["a \\( b \\) c",              2, " b ",      false, 2, 9],
  ["\\[ x \\]",                  0, " x ",      true,  0, 7],
  ["$$x+y$$",                    0, "x+y",      true,  0, 7],
  ["\\begin{equation}\\int f\\end{equation}", 16, "\\int f", true, 0, 36],
  ["\\begin{align*} a \\\\ b \\end{align*}", 15, " a \\\\ b ", true, 0, 34],
];

for (const [src, offset, wantSource, wantDisplay, wantStart, wantEnd] of cases) {
  test(`scanner: ${JSON.stringify(src)}`, () => {
    const r = findMathAt(src, offset);
    assert.ok(r, "expected a math region");
    assert.equal(r!.source, wantSource);
    assert.equal(r!.display, wantDisplay);
    assert.equal(r!.range.start.character, wantStart);
    assert.equal(r!.range.end.character, wantEnd);
  });
}

test("scanner: outside math returns null", () => {
  assert.equal(findMathAt("plain text", 0), null);
});

test("scanner: escaped \\$ is not a delimiter", () => {
  // price is \$5 — the \$ is escaped, only the second $ opens math.
  const src = "price is \\$5 and $x$";
  const r = findMathAt(src, src.indexOf("x"));
  assert.ok(r);
  assert.equal(r!.source, "x");
});

test("scanner: $ after % on same line is ignored (comment)", () => {
  const src = "% $not math$ and $real$ x";
  assert.equal(findMathAt(src, src.indexOf("real")), null);
});

test("scanner: $ inside \\begin{verbatim} block is ignored", () => {
  const src = "\\begin{verbatim}\n$x$\n\\end{verbatim}\nafter block $visible$";
  const r = findMathAt(src, src.indexOf("visible"));
  assert.ok(r, "math after verbatim block should be found");
  assert.equal(r!.source, "visible");
});

test("scanner: skips verbatim", () => {
  assert.equal(findMathAt("\\begin{verbatim}$x$\\end{verbatim}", 16), null);
});

test("scanner: math after inline \\verb|...| is found", () => {
  // Inline \verb is a single command, not a block environment — we don't
  // special-case it.  `$real$` after the \verb|...| is still detected.
  const src = "\\verb|$not$| and $real$ x";
  const r = findMathAt(src, src.indexOf("real"));
  assert.ok(r);
  assert.equal(r!.source, "real");
});

test("scanner: too-long formula returns null", () => {
  const src = "$" + "x".repeat(2000) + "$";
  const r = findMathAt(src, 1, { maxFormulaLength: 500 });
  assert.equal(r, null);
});

test("scanner: adjacent inline $ pairs don't cross-capture", () => {
  const src = "We say $f$ is $\\gamma$-H\\\"{o}lder continuous.\n\\end{definition}\nDefine space $C^{k,\\gamma}(\\O)$ by";
  const r = findMathAt(src, src.indexOf("\\gamma"), { maxFormulaLength: 2000 });
  assert.ok(r, "should find the math region around \\gamma");
  assert.equal(r!.source, "\\gamma");
});

test("scanner: hover on first $ of adjacents finds correct pair", () => {
  const src = "$a$ text $b$";
  const r = findMathAt(src, src.indexOf("a"), { maxFormulaLength: 2000 });
  assert.ok(r);
  assert.equal(r!.source, "a");
});

test("scanner: $$ parity — closing $$ is not mistaken for opener", () => {
  const src = "some display $$x+y$$\n\\begin{definition}\nDefine space $C^{0,\\gamma}(\\O)$ by $$\nmore math\n$$\nwhere $$";
  const offset = src.indexOf("\\gamma");
  const r = findMathAt(src, offset, { maxFormulaLength: 2000 });
  assert.ok(r, "should find the inline math around \\gamma");
  assert.equal(r!.source, "C^{0,\\gamma}(\\O)");
  assert.equal(r!.display, false, "should be inline math, not display");
});

test("scanner: math inside tabular is detected (A3 regression)", () => {
  // tabular cells commonly contain $x$; the scanner must not treat the
  // whole tabular as a verbatim block.
  const src = "\\begin{tabular}{c|c}\n$E=mc^2$ & $x$ \\\\\n\\end{tabular}";
  const r = findMathAt(src, src.indexOf("mc^2"));
  assert.ok(r);
  assert.equal(r!.source, "E=mc^2");
});

test("scanner: math inside tikzpicture node is detected", () => {
  const src = "\\begin{tikzpicture}\n\\node at (0,0) {$x^2$};\n\\end{tikzpicture}";
  const r = findMathAt(src, src.indexOf("x^2"));
  assert.ok(r);
  assert.equal(r!.source, "x^2");
});

test("scanner: range spans full delimiters, not cursor-relative (A5)", () => {
  // Cursor in the middle of $$ ... $$; range must be the whole $$...$$.
  const src = "$$\\alpha + \\beta + \\gamma$$";
  const midOffset = src.indexOf("\\beta");
  const r = findMathAt(src, midOffset);
  assert.ok(r);
  assert.equal(r!.range.start.character, 0);
  assert.equal(r!.range.end.character, src.length);
});

test("scanner: range covers \\begin{equation}..\\end{equation} fully (A5 regression)", () => {
  const src = "intro\n\\begin{equation}\n\\int_0^1 f(x)\\,dx\n\\end{equation}\noutro";
  const r = findMathAt(src, src.indexOf("f(x)"));
  assert.ok(r);
  // Range should start at \begin{equation} on line 1, end after \end{equation}.
  assert.equal(r!.range.start.line, 1);
  assert.equal(r!.range.start.character, 0);
  assert.equal(r!.range.end.line, 3);
  // \end{equation} runs to the end of line 3.
  assert.ok(r!.range.end.character > 0);
});
