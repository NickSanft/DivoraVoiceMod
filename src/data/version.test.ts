import { describe, expect, it } from "vitest";
import { isNewer, parseSemver } from "./version";

describe("parseSemver", () => {
  it("parses plain and v-prefixed triples", () => {
    expect(parseSemver("1.2.3")).toEqual({ major: 1, minor: 2, patch: 3 });
    expect(parseSemver("v1.2.3")).toEqual({ major: 1, minor: 2, patch: 3 });
    expect(parseSemver(" V10.0.4 ")).toEqual({ major: 10, minor: 0, patch: 4 });
    // Ignores pre-release / build suffixes.
    expect(parseSemver("1.2.3-beta.1")).toEqual({ major: 1, minor: 2, patch: 3 });
  });

  it("returns null on malformed input", () => {
    expect(parseSemver("")).toBeNull();
    expect(parseSemver("1.2")).toBeNull();
    expect(parseSemver("latest")).toBeNull();
    expect(parseSemver("v.x.y")).toBeNull();
    // @ts-expect-error defensive on non-strings
    expect(parseSemver(null)).toBeNull();
  });
});

describe("isNewer", () => {
  it("detects newer across each component", () => {
    expect(isNewer("2.0.0", "1.9.9")).toBe(true);
    expect(isNewer("1.3.0", "1.2.9")).toBe(true);
    expect(isNewer("1.2.4", "1.2.3")).toBe(true);
    expect(isNewer("v1.12.0", "1.11.1")).toBe(true);
  });

  it("is false for equal or older", () => {
    expect(isNewer("1.2.3", "1.2.3")).toBe(false);
    expect(isNewer("1.2.3", "v1.2.3")).toBe(false);
    expect(isNewer("1.2.3", "1.3.0")).toBe(false);
    expect(isNewer("1.2.3", "2.0.0")).toBe(false);
  });

  it("is false when either side is malformed (never nag on garbage)", () => {
    expect(isNewer("garbage", "1.2.3")).toBe(false);
    expect(isNewer("1.2.3", "garbage")).toBe(false);
    expect(isNewer("", "")).toBe(false);
    // Dev builds report 0.0.0 — a real release is "newer", but the store
    // guards 0.0.0 out; isNewer itself still computes truthfully here.
    expect(isNewer("1.0.0", "0.0.0")).toBe(true);
  });
});
