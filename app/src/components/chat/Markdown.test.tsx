import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Markdown } from "./Markdown";

describe("Markdown", () => {
  it("renders GitHub tables, emphasis and code", () => {
    const { container } = render(<Markdown text={"**fc** is `1 kHz`\n\n| Spec | Value |\n|---|---|\n| fc | 1.005 kHz |"} />);
    expect(container.querySelector("strong")?.textContent).toBe("fc");
    expect(container.querySelector("code")?.textContent).toBe("1 kHz");
    expect(container.querySelectorAll("td")).toHaveLength(2);
  });

  it("drops raw HTML instead of rendering it", () => {
    const { container } = render(
      <Markdown text={'Hello <script>window.__x = 1</script><img src=x onerror="window.__y = 1"> <b onclick="z()">bold</b>\n\n<iframe src="https://example.com"></iframe>'} />,
    );
    expect(container.querySelector("script, iframe, b")).toBeNull();
    expect(container.innerHTML).not.toMatch(/onerror|onclick/);
    expect((window as unknown as { __y?: number }).__y).toBeUndefined();
  });

  it("removes javascript: links and opens others outside the app", () => {
    const { container } = render(<Markdown text={"[bad](javascript:alert(1)) and [docs](https://ngspice.sourceforge.io)"} />);
    const links = [...container.querySelectorAll("a")];
    expect(links.every((a) => !a.getAttribute("href")?.startsWith("javascript"))).toBe(true);
    const docs = links.find((a) => a.textContent === "docs");
    expect(docs?.getAttribute("target")).toBe("_blank");
    expect(docs?.getAttribute("rel")).toContain("noopener");
  });

  it("does not fetch images from model output", () => {
    const { container } = render(<Markdown text={"![plot](https://tracker.example/pixel.png)"} />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("[image: plot]");
  });
});
