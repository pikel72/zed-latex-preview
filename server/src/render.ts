//! MathJax‑based TeX → SVG rendering.
//!
//! The renderer is initialised once at module load with the full TeX input
//! jax (`AllPackages`) and the SVG output jax.  Each call to `render()`
//! converts a single formula on a throw‑away MathJax document node.
//!
//! ## Size control
//!
//! The `scale` parameter is multiplied into `em` and `ex` so MathJax
//! produces appropriately‑sized glyph paths.  Because the SVG is embedded
//! via `<img src="data:image/svg+xml;…">` there is no parent font to
//! resolve CSS `ex` units — we convert `width` and `height` from `ex` to
//! `px` after rendering, then add a small padding so anti‑aliased glyph
//! edges are not clipped.
//!
//! ## Error detection
//!
//! MathJax marks severe errors with `data-mjx-error` attributes and
//! undefined commands with red‑coloured `<mtext>` nodes.  Both are treated
//! as hard errors so the hover can fall back to showing the raw TeX source.

import { mathjax } from "mathjax-full/js/mathjax.js";
import { TeX } from "mathjax-full/js/input/tex.js";
import { SVG } from "mathjax-full/js/output/svg.js";
import { liteAdaptor } from "mathjax-full/js/adaptors/liteAdaptor.js";
import { RegisterHTMLHandler } from "mathjax-full/js/handlers/html.js";
import { AllPackages } from "mathjax-full/js/input/tex/AllPackages.js";

// ── MathJax globals ────────────────────────────────────────────────────
// Created once and reused.  MathJax.reinit() is not needed because each
// render() call converts to a fresh document node.

const DEFAULT_EM = 16;      // CSS px per em at scale = 1
const DEFAULT_EX = 8;       // CSS px per ex (typically em / 2)

const adaptor = liteAdaptor();             // lightweight DOM adaptor (no browser)
RegisterHTMLHandler(adaptor);

const tex = new TeX({ packages: AllPackages });
const svgOutput = new SVG({ fontCache: "local" });
const doc = mathjax.document("", {
  InputJax: tex,
  OutputJax: svgOutput,
  skipHtmlTags: ["script", "noscript", "style", "textarea", "pre", "code"],
});

// ── helpers ────────────────────────────────────────────────────────────

function colorFor(c: "black" | "white" | "auto"): string {
  if (c === "black") return "black";
  if (c === "white") return "white";
  return "currentColor";  // inherits from CSS (respects light / dark theme)
}

// ── public API ─────────────────────────────────────────────────────────

export interface RenderRequest {
  source: string;
  display: boolean;
  scale: number;
  color: "black" | "white" | "auto";
  timeoutMs: number;
}

export type RenderResult =
  | { ok: true; svg: string }
  | { ok: false; error: string };

/** Render a single LaTeX formula to an SVG string. */
export async function render(req: RenderRequest): Promise<RenderResult> {
  const work = async (): Promise<string> => {
    // Yield to the event loop so the timeout timer can fire.
    await new Promise<void>(r => setImmediate(r));

    const node = doc.convert(req.source, {
      display: req.display,
      em: DEFAULT_EM * req.scale,
      ex: DEFAULT_EX * req.scale,
      containerWidth: 1200 * req.scale,
    });
    const html = adaptor.outerHTML(node);
    const m = html.match(/<svg[\s\S]*?<\/svg>/);
    if (!m) throw new Error("no svg produced");

    // MathJax error markers — surface as errors so the hover shows a
    // readable TeX fallback instead of garbled / red‑text SVG.
    if (m[0].includes("data-mjx-error") || m[0].includes('fill="red"')) {
      throw new Error("mathjax parse error");
    }

    // Inject colour attribute onto the root <svg>.
    let svg = m[0].replace(/<svg([^>]*)>/, `<svg$1 color="${colorFor(req.color)}">`);

    // Convert CSS `ex` → explicit `px` so the image renders at the
    // intended size regardless of the context it is placed in.
    const exPx = DEFAULT_EX * req.scale;
    const PAD = 8;   // anti‑clip padding
    svg = svg.replace(/\bwidth="([\d.]+)ex"/,  (_, w) => `width="${Math.round(Number(w) * exPx) + PAD}px"`);
    svg = svg.replace(/\bheight="([\d.]+)ex"/, (_, h) => `height="${Math.round(Number(h) * exPx) + PAD}px"`);

    return svg;
  };

  try {
    const svg = await withTimeout(work(), req.timeoutMs);
    return { ok: true, svg };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return { ok: false, error: msg };
  }
}

// ── timeout wrapper ────────────────────────────────────────────────────

function withTimeout<T>(p: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`mathjax timeout after ${ms}ms`)), ms);
    p.then(
      v => { clearTimeout(t); resolve(v); },
      e => { clearTimeout(t); reject(e); },
    );
    p.catch(() => {});
  });
}
