import { describe, it, expect } from "vitest";
import { mapError } from "./errors";

describe("mapError (non-custodial)", () => {
  it("maps explorer rate-limit (403 / forbidden) to a busy hint", () => {
    expect(mapError("HNSFans txs lookup failed for hs1q…: status 403 Forbidden")).toMatch(
      /rate-limited/i,
    );
  });

  it("maps explorer/network unreachable to an explorer hint", () => {
    expect(mapError("HNSFans is unreachable at https://e.hnsfans.com")).toMatch(
      /reach the explorer/i,
    );
  });

  it("maps a locked signer to an Unlock hint", () => {
    expect(mapError("Wallet locked")).toMatch(/signer is locked/i);
    expect(mapError("wallet is locked")).toMatch(/signer is locked/i);
  });

  it("maps insufficient funds", () => {
    expect(mapError("insufficient funds")).toContain("Insufficient HNS");
  });

  it("does NOT mention a 'wallet ID' (legacy custodial copy is gone)", () => {
    // A bare 404/"not found" must fall through to the plain message, never the
    // old "Wallet or endpoint not found. Check wallet ID in settings."
    const msg = mapError("status 404 Not Found");
    expect(msg.toLowerCase()).not.toContain("wallet id");
  });

  it("strips technical prefixes", () => {
    const result = mapError("Error invoking remote method 'discover_owned_names': some error");
    expect(result).toBe("some error");
  });

  it("returns original for unknown errors", () => {
    expect(mapError("something completely unknown")).toBe("something completely unknown");
  });

  it("handles Error objects", () => {
    expect(mapError(new Error("status 403 Forbidden"))).toMatch(/rate-limited/i);
  });
});
