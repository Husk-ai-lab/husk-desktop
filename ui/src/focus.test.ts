import { describe, expect, it } from "vitest";
import { isFocusable, parseHash } from "./focus";

describe("parseHash", () => {
  it("reads a tab and a focused card", () => {
    expect(parseHash("#components&focus=studio")).toEqual({ tab: "components", focus: "studio" });
    expect(parseHash("#updates")).toEqual({ tab: "updates", focus: null });
    expect(parseHash("logs&focus=husk")).toEqual({ tab: "logs", focus: "husk" });
  });

  it("falls back to the components tab for anything unknown", () => {
    expect(parseHash("")).toEqual({ tab: "components", focus: null });
    expect(parseHash("#nope")).toEqual({ tab: "components", focus: null });
  });

  it("never accepts a card that is not a component", () => {
    expect(parseHash("#components&focus=../etc")).toEqual({ tab: "components", focus: null });
    expect(parseHash("#components&focus=")).toEqual({ tab: "components", focus: null });
    expect(parseHash("#components&other=studio")).toEqual({ tab: "components", focus: null });
  });

  it("ignores extra parameters and keeps the valid one", () => {
    expect(parseHash("#components&x=1&focus=studio&y=2")).toEqual({ tab: "components", focus: "studio" });
  });
});

describe("isFocusable", () => {
  it("accepts only the two components", () => {
    expect(isFocusable("husk")).toBe(true);
    expect(isFocusable("studio")).toBe(true);
    expect(isFocusable("other")).toBe(false);
    expect(isFocusable(null)).toBe(false);
    expect(isFocusable(undefined)).toBe(false);
  });
});
