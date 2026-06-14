import { test } from "node:test";
import assert from "node:assert/strict";
import { findMathAt } from "../src/scanner.js";

const cases: Array<[string, number, number, string]> = [
  ["let $E = mc^2$.",  4, 13, "E = mc^2"],
  ["a \\( b \\) c",     2,  7, " b "],
  ["\\[ x \\]",        0,  7, " x "],
  ["$$x+y$$",          0,  7, "x+y"],
  ["\\begin{equation}\\int f\\end{equation}", 16, 23, "\\int f"],
  ["\\begin{align*} a \\\\ b \\end{align*}", 15, 30, " a \\\\ b "],
  ["price is \\$5 and $x$", 14, 17, "x"],
  ["\\verb|$not$| and $real$ x", 18, 23, "real"],
];

for (const [src, start, end, want] of cases) {
  test(`scanner: ${JSON.stringify(src)}`, () => {
    const r = findMathAt(src, start);
    assert.ok(r, "expected a math region");
    assert.equal(r!.source, want);
    assert.equal(r!.range.start.character, start);
    assert.equal(r!.range.end.character, end);
  });
}

test("scanner: outside math returns null", () => {
  assert.equal(findMathAt("plain text", 0), null);
});

test("scanner: $ after % on same line is ignored (comment)", () => {
  // Everything after `%` is a comment — no math should be detected.
  assert.equal(findMathAt("% $not math$ and $real$ x", 18), null);
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

test("scanner: too-long formula returns null", () => {
  const src = "$" + "x".repeat(2000) + "$";
  const r = findMathAt(src, 1, { maxFormulaLength: 500 });
  assert.equal(r, null);
});

test("scanner: adjacent $ pairs don't cross-capture", () => {
  // Bug: hovering over γ in $f$ is $\gamma$-Hölder should not capture
  // everything up to $C^{k,\gamma}$.
  const src = "We say $f$ is $\\gamma$-H\\\"{o}lder continuous.\n\\end{definition}\nDefine space $C^{k,\\gamma}(\\O)$ by";
  const r = findMathAt(src, src.indexOf("\\gamma"), { maxFormulaLength: 2000 });
  assert.ok(r, "should find the math region around \\gamma");
  assert.equal(r!.source, "\\gamma");
});

test("scanner: hover on first $ of adjacents finds correct pair", () => {
  const src = "$a$ text $b$";
  // Cursor on 'a'
  const r = findMathAt(src, src.indexOf("a"), { maxFormulaLength: 2000 });
  assert.ok(r);
  assert.equal(r!.source, "a");
});

test("scanner: $$ parity — closing $$ is not mistaken for opener", () => {
  // Reproduces the bug from functional_1.tex line 62:
  //   ...$$              <-- closing $$ of previous display math
  //   \begin{definition}
  //   Define space $C^{0,\gamma}(\O)$ by $$
  // Hovering on \gamma in the inline $...$ should find C^{0,\gamma}(\O),
  // NOT the closing $$ of the previous block.
  const src = "some display $$x+y$$\n\\begin{definition}\nDefine space $C^{0,\\gamma}(\\O)$ by $$\nmore math\n$$\nwhere $$";
  const offset = src.indexOf("\\gamma");
  const r = findMathAt(src, offset, { maxFormulaLength: 2000 });
  assert.ok(r, "should find the inline math around \\gamma");
  assert.equal(r!.source, "C^{0,\\gamma}(\\O)");
  assert.equal(r!.display, false, "should be inline math, not display");
});
