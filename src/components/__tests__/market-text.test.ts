import { describe, it, expect } from "vitest";
import { listedUntilText } from "../market/marketText";

describe("listedUntilText", () => {
  it("dates the seller's expiresAt in UTC, and says the date does not end the listing", () => {
    expect(listedUntilText(1815232480)).toBe(
      "Listed until July 10, 2027 · stays buyable until sold or cancelled",
    );
  });

  it("gives no text for an expiresAt no date can hold, rather than throwing", () => {
    // expiresAt comes from a foreign listing file as any u64.
    expect(listedUntilText(Number.MAX_SAFE_INTEGER)).toBeNull();
    expect(listedUntilText(18446744073709552000)).toBeNull();
  });
});
