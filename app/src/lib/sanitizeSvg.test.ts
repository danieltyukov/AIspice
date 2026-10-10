import { afterEach, describe, expect, it } from "vitest";
import { sanitizeSvg } from "./sanitizeSvg";

const wrap = (body: string, attrs = "") => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10" ${attrs}>${body}</svg>`;

/** Mounts sanitised output the way the app does and reports anything live left in it. */
function mount(markup: string): HTMLDivElement {
  const host = document.createElement("div");
  host.innerHTML = sanitizeSvg(markup);
  document.body.appendChild(host);
  return host;
}

function liveBits(host: HTMLElement): string[] {
  const found: string[] = [];
  for (const el of host.querySelectorAll("*")) {
    const tag = el.localName.toLowerCase();
    if (["script", "foreignobject", "style", "a", "use", "image", "iframe", "animate", "set", "animatemotion", "animatetransform", "img"].includes(tag)) {
      found.push(`<${tag}>`);
    }
    for (const attr of el.attributes) {
      const name = attr.name.toLowerCase();
      if (name.startsWith("on") || name === "href" || name.endsWith(":href") || name === "style") found.push(`${tag}@${name}`);
      if (/javascript:|data:|@import|expression\(/i.test(attr.value)) found.push(`${tag}@${name}=${attr.value}`);
    }
  }
  return found;
}

afterEach(() => {
  document.body.innerHTML = "";
  delete (window as unknown as { __pwned?: number }).__pwned;
});

describe("sanitizeSvg", () => {
  it("keeps the drawing, its classes and data attributes", () => {
    const host = mount(
      wrap(
        '<g class="sch-part" data-inst="R1" data-symbol="res"><rect class="sch-hit" x="1" y="1" width="2" height="2"/>' +
          '<path class="sch-symbol" d="M0 0L5 5"/></g><polyline class="wire" data-wire="w1" points="0,0 5,5"/>' +
          '<text class="sch-label" text-anchor="middle">out</text><rect fill="url(#grad)" width="1" height="1"/>',
      ),
    );
    const svg = host.querySelector("svg")!;
    expect(svg.getAttribute("viewBox")).toBe("0 0 10 10");
    expect(host.querySelector('[data-inst="R1"]')?.getAttribute("data-symbol")).toBe("res");
    expect(host.querySelector('[data-wire="w1"]')?.getAttribute("points")).toBe("0,0 5,5");
    expect(host.querySelector(".sch-symbol")?.getAttribute("d")).toBe("M0 0L5 5");
    expect(host.querySelector(".sch-label")?.textContent).toBe("out");
    expect(host.querySelector('[fill="url(#grad)"]')).not.toBeNull();
  });

  // Known bypass payloads. Each must come out inert.
  const payloads: Array<[string, string]> = [
    ["javascript: link", '<svg><a href="javascript:alert(1)"><text>x</text></a></svg>'],
    ["foreignObject with HTML", "<svg><foreignObject><img src=x onerror=alert(1)></foreignObject></svg>"],
    ["animate rewriting href", '<svg><animate attributeName="href" to="javascript:alert(1)"/></svg>'],
    ["set rewriting an attribute", '<svg><rect width="1" height="1"><set attributeName="onclick" to="alert(1)"/></rect></svg>'],
    ["use with a data: URL", '<svg><use href="data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9YWxlcnQoMSk+PC9zdmc+"/></svg>'],
    ["style with @import", "<svg><style>@import url(https://evil.example/x.css); .button{display:none}</style></svg>"],
    ["onload on the root", "<svg onload=alert(1)><rect width=1 height=1 /></svg>"],
    ["mixed-case script", "<svg><SCRIPT>window.__pwned = 1</SCRIPT><ScRiPt>window.__pwned = 1</ScRiPt></svg>"],
    ["xlink:href", '<svg xmlns:xlink="http://www.w3.org/1999/xlink"><use xlink:href="https://evil.example/s.svg#a"/><a xlink:href="javascript:alert(1)">x</a></svg>'],
    ["image from the network", '<svg><image href="https://evil.example/pixel.png" width="1" height="1"/></svg>'],
    ["entity-encoded javascript", '<svg><a href="&#106;avascript:alert(1)"><text>x</text></a></svg>'],
    ["style attribute fetching", '<svg><rect style="fill:url(https://evil.example/t)" width="1" height="1"/></svg>'],
    ["presentation attribute fetching", '<svg><rect fill="url(https://evil.example/t)" filter="url(data:x)" width="1" height="1"/></svg>'],
    ["expression in style", '<svg><rect style="width: expression(alert(1))"/></svg>'],
    ["iframe smuggled in", '<svg><iframe src="javascript:alert(1)"></iframe></svg>'],
    ["event handlers everywhere", '<svg><g onclick="x()" onmouseover="y()"><circle r="1" onfocus="z()" tabindex="0"/></g></svg>'],
  ];

  for (const [name, payload] of payloads) {
    it(`neutralises ${name}`, () => {
      const host = mount(payload);
      expect(liveBits(host)).toEqual([]);
      expect((window as unknown as { __pwned?: number }).__pwned).toBeUndefined();
    });
  }

  it("returns nothing for markup without an SVG", () => {
    expect(sanitizeSvg("")).toBe("");
    expect(sanitizeSvg("<div>hi</div>")).toBe("");
    expect(sanitizeSvg("<script>alert(1)</script>")).toBe("");
  });

  it("keeps an XML prolog and comments out of the result", () => {
    const out = sanitizeSvg('<?xml version="1.0"?><!-- note --><svg viewBox="0 0 1 1"><rect width="1" height="1"/></svg>');
    expect(out.startsWith("<svg")).toBe(true);
    expect(out).not.toContain("note");
  });
});
