import { describe, expect, it } from "vitest";
import { toAceName } from "./idnEncode";
import { readsAsLatin } from "./confusable";

const ace = toAceName;

describe("readsAsLatin", () => {
  it("reads a mixed-script name as the name it imitates", () => {
    expect(readsAsLatin(ace("аpple"))).toBe("apple"); // Cyrillic а
    expect(readsAsLatin(ace("gооgle"))).toBe("google"); // Cyrillic о
    expect(readsAsLatin(ace("pαypal"))).toBe("paypal"); // Greek α
  });

  it("reads a name written only in look-alikes too", () => {
    expect(readsAsLatin(ace("асе"))).toBe("ace");
  });

  it("says nothing of a plain name", () => {
    expect(readsAsLatin("apple")).toBeNull();
    expect(readsAsLatin("xn--")).toBeNull();
  });

  it("says nothing of an IDN with a letter that gives it away", () => {
    expect(readsAsLatin(ace("сбер"))).toBeNull(); // б
    expect(readsAsLatin(ace("münchen"))).toBeNull();
    expect(readsAsLatin(ace("中文"))).toBeNull();
  });
});
