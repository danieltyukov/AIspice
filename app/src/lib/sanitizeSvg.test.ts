import { describe, expect, it } from "vitest";
import { sanitizeSvg } from "./sanitizeSvg";

const wrap = (body: string, attrs = "") => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" ${attrs}>${body}</svg>`;

function parse(markup: string): Document {
  return new DOMParser().parseFromString(markup, "image/svg+xml");
}

describe("sanitizeSvg", () => {
  it("keeps the drawing, its classes and data-inst", () => {
    const out = sanitizeSvg(
      wrap('<g class="sch-part" data-inst="R1"><rect class="sch-hit" x="1" y="1" width="2" height="2"/><path class="sch-symbol" d="M0 0L5 5"/></g><text class="sch-label">out</text>'),
    );
    const doc = parse(out);
    expect(doc.querySelector('[data-inst="R1"]')).not.toBeNull();
    expect(doc.querySelector(".sch-symbol")?.getAttribute("d")).toBe("M0 0L5 5");
    expect(doc.querySelector(".sch-label")?.textContent).toBe("out");
    expect(doc.documentElement.getAttribute("viewBox")).toBe("0 0 10 10");
  });

  it("removes scripts, foreign content, style sheets and animation", () => {
    const out = sanitizeSvg(
      wrap(
        '<script>alert(1)</script><foreignObject><div>x</div></foreignObject><style>.button{display:none}</style>' +
          '<rect width="1" height="1"><set attributeName="onclick" to="alert(1)"/><animate attributeName="href" values="javascript:alert(1)"/></rect>',
      ),
    );
    expect(out).not.toMatch(/script|foreignObject|<style|<set|<animate|alert/i);
    expect(out).toContain("<rect");
  });

  it("strips every event handler attribute, including on the root", () => {
    const out = sanitizeSvg(wrap('<g onclick="alert(1)" onmouseover="x()"><circle r="1" onload="y()"/></g>', 'onload="z()"'));
    expect(out).not.toMatch(/\son\w+=/i);
    expect(out).toContain("<circle");
  });

  it("drops links and urls that leave the document", () => {
    const out = sanitizeSvg(
      wrap(
        '<a href="javascript:alert(1)"><text>a</text></a>' +
          '<use href="https://evil.example/x.svg#a"/><use href="#local"/>' +
          '<image href="http://evil.example/p.png"/><image href="data:image/png;base64,iVBORw0KGgo="/>' +
          '<rect style="fill:url(https://evil.example/t)" width="1" height="1"/><rect fill="url(#grad)" width="1" height="1"/>',
      ),
    );
    expect(out).not.toMatch(/javascript:|evil\.example/);
    expect(out).toContain('href="#local"');
    expect(out).toContain("data:image/png;base64");
    expect(out).toContain('fill="url(#grad)"');
  });

  it("returns nothing for markup that is not an SVG", () => {
    expect(sanitizeSvg("")).toBe("");
    expect(sanitizeSvg("<div>hi</div>")).toBe("");
    expect(sanitizeSvg("<svg><g></svg")).toBe("");
  });
});
