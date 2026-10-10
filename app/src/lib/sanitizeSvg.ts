/*
 * The schematic and plot SVGs come from our own backend, but they are built
 * from files on disk and from model output, so they are treated as untrusted
 * before they reach innerHTML.
 *
 * Removed: scripts, foreign content, style sheets (they would apply to the
 * whole app), animation elements (they can rewrite attributes), every event
 * handler attribute, and any link or url() that leaves the document. Styling
 * comes from the app's own CSS through the sch-* classes.
 */

const DROP = new Set([
  "script",
  "foreignobject",
  "style",
  "iframe",
  "object",
  "embed",
  "link",
  "meta",
  "base",
  "animate",
  "animatemotion",
  "animatetransform",
  "set",
  "discard",
  "handler",
  "listener",
  "audio",
  "video",
  "canvas",
]);

const URL_ATTRS = new Set(["href", "xlink:href", "src", "action", "formaction"]);

function isSafeUrl(value: string, tag: string): boolean {
  const v = value.trim();
  if (v.startsWith("#")) return true;
  // Embedded raster images inside a plot are fine; nothing that fetches.
  if (tag === "image" && /^data:image\/(png|jpeg|gif|webp);base64,[a-z0-9+/=\s]*$/i.test(v)) return true;
  return false;
}

function isSafeStyle(value: string): boolean {
  const v = value.toLowerCase();
  if (v.includes("expression(") || v.includes("javascript:") || v.includes("@import") || v.includes("behavior:")) {
    return false;
  }
  // url(#id) references a gradient or marker in the same SVG; anything else fetches.
  const urls = v.match(/url\(([^)]*)\)/g) ?? [];
  return urls.every((u) => /^url\(\s*['"]?#/.test(u));
}

function cleanElement(el: Element): void {
  for (const attr of [...el.attributes]) {
    const name = attr.name.toLowerCase();
    const value = attr.value;
    if (name.startsWith("on")) {
      el.removeAttribute(attr.name);
    } else if (URL_ATTRS.has(name)) {
      if (!isSafeUrl(value, el.localName.toLowerCase())) el.removeAttribute(attr.name);
    } else if (name === "style") {
      if (!isSafeStyle(value)) el.removeAttribute(attr.name);
    } else if (/^\s*(javascript|vbscript|data):/i.test(value)) {
      el.removeAttribute(attr.name);
    } else if (/url\(/i.test(value) && !isSafeStyle(value)) {
      el.removeAttribute(attr.name);
    }
  }
}

/** Returns sanitised SVG markup, or an empty string when the input is not an SVG. */
export function sanitizeSvg(markup: string): string {
  if (!markup || typeof DOMParser === "undefined") return "";
  const doc = new DOMParser().parseFromString(markup, "image/svg+xml");
  const root = doc.documentElement;
  if (!root || root.localName.toLowerCase() !== "svg" || doc.getElementsByTagName("parsererror").length > 0) {
    return "";
  }

  const walk = (el: Element): void => {
    for (const child of [...el.children]) {
      if (DROP.has(child.localName.toLowerCase())) {
        child.remove();
        continue;
      }
      cleanElement(child);
      walk(child);
    }
  };
  cleanElement(root);
  walk(root);

  // Processing instructions and comments carry nothing we draw.
  const strip = (node: Node): void => {
    for (const child of [...node.childNodes]) {
      if (child.nodeType === 7 || child.nodeType === 8) child.remove();
      else strip(child);
    }
  };
  strip(root);

  return new XMLSerializer().serializeToString(root);
}
