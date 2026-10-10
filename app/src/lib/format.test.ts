import { describe, expect, it } from "vitest";
import { baseName, duration, eng, plural, relative, spice, spiceTicks, vectorUnit } from "./format";

describe("spice", () => {
  it("writes SPICE suffixes", () => {
    expect(spice(1000)).toBe("1k");
    expect(spice(10_000)).toBe("10k");
    expect(spice(1e6)).toBe("1Meg");
    expect(spice(2200)).toBe("2.2k");
    expect(spice(4.7e-6)).toBe("4.7u");
    expect(spice(1.6e-7)).toBe("160n");
    expect(spice(1e-12)).toBe("1p");
    expect(spice(3e9)).toBe("3G");
    expect(spice(0.5)).toBe("500m");
    expect(spice(-0.25)).toBe("-250m");
    expect(spice(0)).toBe("0");
    expect(spice(42)).toBe("42");
  });

  it("survives float noise at a boundary", () => {
    expect(spice(999.99999999)).toBe("1k");
    expect(spice(0.1 + 0.2)).toBe("300m");
  });
});

describe("spiceTicks", () => {
  it("labels a decade axis", () => {
    expect(spiceTicks([10, 100, 1000, 10_000, 100_000, 1e6])).toEqual(["10", "100", "1k", "10k", "100k", "1Meg"]);
  });

  it("adds precision until neighbours differ", () => {
    expect(spiceTicks([1000, 1001, 1002])).toEqual(["1k", "1.001k", "1.002k"]);
  });

  it("snaps near-zero ticks to 0", () => {
    expect(spiceTicks([-0.001, 1e-19, 0.001])).toEqual(["-1m", "0", "1m"]);
  });

  it("labels a time axis", () => {
    expect(spiceTicks([0, 0.0005, 0.001, 0.0015])).toEqual(["0", "500u", "1m", "1.5m"]);
  });
});

describe("eng", () => {
  it("formats readouts with SI prefixes and units", () => {
    expect(eng(1005.3, "Hz")).toBe("1.005 kHz");
    expect(eng(3.52e-4, "s")).toBe("352.0 µs");
    expect(eng(-0.0912, "dB")).toBe("-0.09 dB");
    expect(eng(-45, "deg")).toBe("-45.00 °");
    expect(eng(null, "V")).toBe("n/a");
    expect(eng(0, "A")).toBe("0 A");
  });
});

describe("duration and relative time", () => {
  it("formats durations", () => {
    expect(duration(42)).toBe("42 ms");
    expect(duration(412)).toBe("0.4 s");
    expect(duration(12_400)).toBe("12 s");
    expect(duration(125_000)).toBe("2 min 5 s");
  });

  it("formats relative times", () => {
    const now = Date.UTC(2026, 9, 10, 12);
    expect(relative(now - 10_000, now)).toBe("just now");
    expect(relative(now - 12 * 60_000, now)).toBe("12 min ago");
    expect(relative(now - 3 * 3_600_000, now)).toBe("3 h ago");
    expect(relative(now - 30 * 3_600_000, now)).toBe("yesterday");
    expect(relative(now - 3 * 86_400_000, now)).toBe("3 days ago");
  });
});

describe("small helpers", () => {
  it("pluralises, names files and infers units", () => {
    expect(plural(1, "spec")).toBe("1 spec");
    expect(plural(3, "spec")).toBe("3 specs");
    expect(baseName("sub/dir/rc.asc")).toBe("rc.asc");
    expect(vectorUnit("V(out)")).toBe("V");
    expect(vectorUnit("I(R1)")).toBe("A");
    expect(vectorUnit("Ic(Q1)")).toBe("A");
    expect(vectorUnit("frequency")).toBe("Hz");
  });
});
