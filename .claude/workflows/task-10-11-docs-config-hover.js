export const meta = {
  name: 'task-10-11-docs-config-hover',
  description: 'Implement document store, config, and hover handler',
  phases: [
    { title: 'Implement' },
    { title: 'Spec review' },
    { title: 'Quality review' },
  ],
}

phase('Implement')
const result = await agent(`You are implementing Tasks 10 and 11 of the Zed LaTeX hover preview LSP server.

## Context

You are in \`X:/Code/zed-extensions/extensions/latex/server/\` (TS LSP server, ES modules, Node 18+).

The project already has:
- \`src/scanner.ts\` — \`findMathAt(text, offset, opts)\`, \`offsetToPosition\`
- \`src/macros.ts\` — \`extractMacros\`, \`expand\`
- \`src/render.ts\` — \`RenderRequest\`, \`RenderResult\`, \`render(req)\`
- \`src/cache.ts\` — \`CacheKey\`, \`LRU<V>\`
- 32 tests passing across all suites (12 scanner + 4 macros + 8 render smoke + 8 cache)

## Plan reference

Read both task blocks in \`X:/Code/docs/superpowers/plans/2026-06-13-zed-latex-hover-preview.md\`:
- Task 10 (Document store and config plumbing): lines 692–748
- Task 11 (Hover handler): lines 751–874

## Your job — Task 10

Create two files:
- \`src/documents.ts\` — \`DocumentStore\` class with \`open\`, \`change\`, \`close\`, \`get\` methods backed by a \`Map<string, string>\`.
- \`src/config.ts\` — \`PreviewConfig\` interface, \`defaultConfig()\`, \`configFromInit(init)\` that merges initialization options onto defaults with type guards.

## Your job — Task 11

Create two files:
- \`src/hover.ts\` — \`hoverFor(text, position, cfg)\` that orchestrates: scanner → macro extract/expand → cache check → render → data URI encoding → markdown. The cache is module-scope (256 entries). If render fails, returns a fenced LaTeX code block as fallback.
- \`test/hover.test.ts\` — 4 tests from the spec:
  - hover on $E=mc^2$ returns markdown image
  - hover outside math returns null
  - hover on macro \\R^2 returns markdown image
  - hover on broken math returns TeX fallback

## Verify

- \`cd X:/Code/zed-extensions/extensions/latex/server && npx tsc -p tsconfig.json\` exits 0
- \`npm test\` → 36 tests pass (32 prior + 4 hover)

## Constraints

- The spec's \`hoverFor\` uses \`positionToOffset(text, position)\` to convert LSP Position to text offset. The scanner returns ranges in {line, character}. Pass the right thing to \`findMathAt\`.
- Use \`import { ... } from "../src/X.js"\` (ESM with .js extension).
- Match the spec code exactly. Only deviate if there's a clear correctness issue — note it.
- Do not commit.
- Do not add comments or restructure beyond the spec.

## Report

- What you built
- Any deviations from the spec
- Test count
- Status: DONE / DONE_WITH_CONCERNS / BLOCKED`, {label: 'Task 10+11 implementer', phase: 'Implement'})

if (!result) return { status: 'NO_RESULT' }

phase('Spec review')
const specVerdict = await agent(`You are reviewing Tasks 10 (documents, config) and 11 (hover) of the Zed LaTeX hover preview LSP server for SPEC COMPLIANCE.

## What to review

- \`X:/Code/zed-extensions/extensions/latex/server/src/documents.ts\`
- \`X:/Code/zed-extensions/extensions/latex/server/src/config.ts\`
- \`X:/Code/zed-extensions/extensions/latex/server/src/hover.ts\`
- \`X:/Code/zed-extensions/extensions/latex/server/test/hover.test.ts\`

## Spec

Read both task blocks in \`X:/Code/docs/superpowers/plans/2026-06-13-zed-latex-hover-preview.md\`:
- Task 10: lines 692–748
- Task 11: lines 751–874

## Your job

1. Verify documents.ts matches the spec: \`DocumentStore\` class with the 4 methods backed by \`Map<string, string>\`.
2. Verify config.ts matches the spec: \`PreviewConfig\` interface, \`defaultConfig()\`, \`configFromInit()\` with type-guarded merging.
3. Verify hover.ts matches the spec: \`hoverFor()\` orchestration, module-scope cache, data URI encoding, fallback to fenced LaTeX on error.
4. Verify hover.test.ts matches all 4 spec test cases.
5. Run \`cd X:/Code/zed-extensions/extensions/latex/server && npm test\` to confirm 36 tests pass (32 prior + 4 hover).
6. Report any deviations.

Return **Spec compliance: ✅ or ❌** with issues.`, {label: 'Task 10+11 spec review', phase: 'Spec review'})

if (!specVerdict) return { status: 'NO_REVIEW' }

phase('Quality review')
const qualityVerdict = await agent(`You are reviewing Tasks 10 (documents, config) and 11 (hover) of the Zed LaTeX hover preview LSP server for CODE QUALITY.

## What to review

- \`X:/Code/zed-extensions/extensions/latex/server/src/documents.ts\`
- \`X:/Code/zed-extensions/extensions/latex/server/src/config.ts\`
- \`X:/Code/zed-extensions/extensions/latex/server/src/hover.ts\`
- \`X:/Code/zed-extensions/extensions/latex/server/test/hover.test.ts\`

## Context

This is the orchestration layer that ties scanner → macros → cache → render. It's called on every hover in a LaTeX buffer. Position conversions between LSP (line, character) and text offset are critical.

## Your job

1. Read all four files.
2. Assess:
   - **Position conversion**: Is \`positionToOffset\` correct? Edge cases: end of document, empty text, character past EOL, line beyond end?
   - **Cache key**: Does the hover code use \`expanded\` source AND macroBlock? Does it include the scale/theme? Are the keys correct?
   - **Type safety**: Any unsafe casts? Any \`as any\`?
   - **Error handling**: What if \`render\` throws synchronously? What if the text is huge? What if a URI contains characters that need encoding for the data URI? (For \`data:image/svg+xml;base64,...\` with base64 encoding, the SVG content is base64-encoded so no URI escaping is needed — but verify.)
   - **Test coverage**: Are the 4 hover tests sufficient? Missing: scale passed through, color passed through, disabled config returns null, macroBlock changes invalidate cache, return value's \`range\` is present and correct.
   - **Performance**: \`extractMacros\` runs on the entire document on every hover — is that wasteful? Should it be cached per document?

Return **Code quality: ✅ Approved** or **❌ Issues** with Critical/Important/Minor.`, {label: 'Task 10+11 quality review', phase: 'Quality review'})

return {
  implementer: result,
  spec: specVerdict,
  quality: qualityVerdict,
}
