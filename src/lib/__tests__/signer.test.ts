import { describe, it, expect } from "vitest";
import { isSignerUnlockedFor } from "../signer";

const session = (walletProfileId: string | null, unlocked = true) => ({
  walletProfileId,
  unlocked,
  unlockedUntilEpochMs: unlocked ? 1 : 0,
});

describe("isSignerUnlockedFor", () => {
  it("is true only for the profile the session belongs to", () => {
    expect(isSignerUnlockedFor(session("a"), "a")).toBe(true);
    expect(isSignerUnlockedFor(session("a"), "b")).toBe(false);
  });

  it("is false when locked, missing, or there is no active profile", () => {
    expect(isSignerUnlockedFor(session(null, false), "a")).toBe(false);
    expect(isSignerUnlockedFor(undefined, "a")).toBe(false);
    expect(isSignerUnlockedFor(session("a"), null)).toBe(false);
    expect(isSignerUnlockedFor(session("a"), undefined)).toBe(false);
  });
});
