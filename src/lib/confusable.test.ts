import { describe, expect, it } from "vitest";
import { toAceName } from "./idnEncode";
import { confusable } from "./confusable";

const readsAs = (name: string) => confusable(name)?.readsAs ?? null;

describe("confusable", () => {
  it("reads a mixed-script name as the name it imitates", () => {
    expect(readsAs(toAceName("аpple"))).toBe("apple"); // Cyrillic а
    expect(readsAs(toAceName("gооgle"))).toBe("google"); // Cyrillic о
    expect(readsAs(toAceName("pαypal"))).toBe("paypal"); // Greek α
  });

  it("reads a name written only in look-alikes too", () => {
    expect(readsAs(toAceName("асе"))).toBe("ace");
  });

  it("says nothing of a plain name", () => {
    expect(confusable("apple")).toBeNull();
    expect(confusable("xn--")).toBeNull();
  });

  it("says nothing of an IDN with a letter that gives it away", () => {
    expect(confusable(toAceName("сбер"))).toBeNull(); // б
    expect(confusable(toAceName("münchen"))).toBeNull();
    expect(confusable(toAceName("中文"))).toBeNull();
  });

  // Built with a raw punycode encoder: `toAceName` runs IDNA, which removes
  // these, while the chain takes any label.
  it("a hidden character does not take a look-alike out of the table", () => {
    expect(confusable("xn--pple-43d4093b")).toEqual({ readsAs: "apple", hidden: true }); // а, ZWSP
    expect(confusable("xn--pypal-4ve9401c")).toEqual({ readsAs: "paypal", hidden: true }); // а, ZWJ
    expect(confusable("xn--pple-tga305c")).toEqual({ readsAs: "apple", hidden: true }); // а, soft hyphen
    expect(confusable("xn--apple-pt3b")).toEqual({ readsAs: "apple", hidden: true }); // ZWSP alone
  });

  it("a compatibility form reads as its letter", () => {
    expect(confusable("xn--pple-zg0y")).toEqual({ readsAs: "apple", hidden: false }); // fullwidth a
  });

  it("a hidden character is told even when the letters give the name away", () => {
    expect(confusable("xn--90ai7ab7866a")).toEqual({ readsAs: null, hidden: true }); // сбер, ZWSP
  });
});
