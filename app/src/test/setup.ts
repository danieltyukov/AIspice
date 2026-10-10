import { configure } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  cleanup();
  try {
    localStorage.clear();
  } catch {
    // jsdom always has storage; this only guards odd environments.
  }
  document.documentElement.removeAttribute("data-theme");
});

// jsdom lacks these; components feature-detect, but some libraries do not.
if (typeof window !== "undefined" && !window.matchMedia) {
  Object.defineProperty(window, "matchMedia", {
    value: (query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }),
  });
}

// A scripted agent turn streams several steps; under parallel test files it
// can take longer than the 1 s default, so asynchronous queries wait longer.
configure({ asyncUtilTimeout: 8000 });
