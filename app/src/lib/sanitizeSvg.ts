import DOMPurify from "dompurify";

/*
 * Schematic and plot SVGs come from our own backend, but they are built from
 * files on disk and from model output, so they are treated as untrusted
 * before they reach the DOM. DOMPurify does the work with its SVG allowlist;
 * on top of it we forbid everything that can link, embed, animate an
 * attribute or carry a style sheet:
 *
 *   - no <a>, <use>, <image>, <foreignObject>, <script>, <iframe>, no
 *     animation elements, and no <style> (inline SVG styles are global to the
 *     page; schematic colours come from the app's CSS through classes)
 *   - no href or xlink:href at all
 *   - any remaining attribute whose value points outside the document
 *     (url() to anything but #id, javascript:, data:, expression(), @import)
 *     is dropped by a hook
 *
 * data-* attributes stay, because the drawing marks parts with data-inst and
 * friends (data-symbol, data-wire).
 */

const FORBID_TAGS = [
  "foreignObject",
  "script",
  "animate",
  "set",
  "animateMotion",
  "animateTransform",
  "image",
  "use",
  "a",
  "iframe",
  "style",
  "discard",
];

const FORBID_ATTR = ["href", "xlink:href", "style"];

const UNSAFE_VALUE = /javascript:|vbscript:|data:|expression\s*\(|@import|behavior\s*:/i;

function unsafeValue(value: string): boolean {
  if (UNSAFE_VALUE.test(value)) return true;
  // url(#id) references a gradient or marker in the same SVG; anything else fetches.
  const urls = value.match(/url\s*\(([^)]*)\)/gi) ?? [];
  return urls.some((u) => !/^url\s*\(\s*['"]?#/i.test(u));
}

let hooked = false;
function ensureHook(): void {
  if (hooked) return;
  hooked = true;
  DOMPurify.addHook("uponSanitizeAttribute", (_node, data) => {
    if (unsafeValue(data.attrValue)) data.keepAttr = false;
  });
}

/**
 * Returns the sanitised SVG as a live element, ready to append. Components use
 * this rather than the string form, so the cleaned tree goes into the page as
 * is and is never serialised and parsed a second time.
 */
export function sanitizeSvgElement(markup: string): SVGSVGElement | null {
  if (!markup || !DOMPurify.isSupported) return null;
  ensureHook();
  const fragment = DOMPurify.sanitize(markup, {
    USE_PROFILES: { svg: true, svgFilters: false },
    FORBID_TAGS,
    FORBID_ATTR,
    ALLOW_DATA_ATTR: true,
    RETURN_DOM_FRAGMENT: true,
  });
  return fragment.querySelector("svg");
}

/** The same, as markup; an empty string when the input holds no SVG. */
export function sanitizeSvg(markup: string): string {
  return sanitizeSvgElement(markup)?.outerHTML ?? "";
}
