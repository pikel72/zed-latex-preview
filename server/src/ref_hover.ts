//! Placeholder for `\ref{...}` hover previews.
//!
//! Real ref-hover lands in Phase 2 (per `docs/plan-ref-cite-hover.md`
//! Section 9).  For Phase 1 the dispatcher in `hover.ts` calls into this
//! file and gets `null`, so the cursor falls through to the math path.

import type { LabelRef } from "./rpc_types.js";

export function refHoverFor(
  _result: { found: true; entry: LabelRef } | { found: false },
  _range?: [number, number],
): { contents: { kind: "markdown"; value: string } } | null {
  return null;
}
