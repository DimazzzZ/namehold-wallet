import { describe, it, expect } from "vitest";
import {
  auctionPhase,
  nextTransition,
  formatCountdown,
  recommendedAction,
  auctionGuidance,
  hnsToDoos,
  doosToHns,
} from "./auction";
import type { HsdNameStats } from "../types";

describe("auctionPhase", () => {
  it("maps known states to labelled badges", () => {
    expect(auctionPhase("OPENING")).toMatchObject({ phase: "OPENING", label: "Opening" });
    expect(auctionPhase("BIDDING")).toMatchObject({ phase: "BIDDING", variant: "warning" });
    expect(auctionPhase("REVEAL")).toMatchObject({ phase: "REVEAL", variant: "warning" });
    expect(auctionPhase("CLOSED")).toMatchObject({ phase: "CLOSED", variant: "success" });
  });

  it("is case-insensitive and treats blank/AVAILABLE as available", () => {
    expect(auctionPhase("bidding").phase).toBe("BIDDING");
    expect(auctionPhase("").phase).toBe("AVAILABLE");
    expect(auctionPhase(null).phase).toBe("AVAILABLE");
  });

  it("passes through unknown states as OTHER", () => {
    expect(auctionPhase("WAT")).toMatchObject({ phase: "OTHER", label: "WAT" });
  });
});

describe("nextTransition", () => {
  it("picks the countdown for the current phase", () => {
    const stats: HsdNameStats = { blocksUntilReveal: 12, hoursUntilReveal: 2 };
    expect(nextTransition("BIDDING", stats)).toEqual({
      label: "Reveal starts in",
      blocks: 12,
      hours: 2,
    });
  });

  it("uses blocksUntilClose during REVEAL", () => {
    expect(nextTransition("REVEAL", { blocksUntilClose: 3 })).toMatchObject({
      label: "Auction closes in",
      blocks: 3,
    });
  });

  it("returns null when the relevant stat is missing or stats absent", () => {
    expect(nextTransition("BIDDING", { blocksUntilClose: 3 })).toBeNull();
    expect(nextTransition("BIDDING", null)).toBeNull();
    expect(nextTransition("CLOSED", { blocksUntilExpire: 100 })).toMatchObject({ blocks: 100 });
  });
});

describe("formatCountdown", () => {
  it("formats blocks + an hours/minutes hint", () => {
    expect(formatCountdown({ label: "x", blocks: 12, hours: 2 })).toBe("12 blocks (~2h)");
    expect(formatCountdown({ label: "x", blocks: 1, hours: 0.1 })).toBe("1 block (~6m)");
    expect(formatCountdown({ label: "x", blocks: 5, hours: null })).toBe("5 blocks");
  });
});

describe("recommendedAction", () => {
  it("recommends the phase-appropriate action", () => {
    expect(recommendedAction("AVAILABLE")?.key).toBe("OPEN");
    expect(recommendedAction("BIDDING")?.key).toBe("BID");
    expect(recommendedAction("REVEAL")?.key).toBe("REVEAL");
    expect(recommendedAction("CLOSED")?.key).toBe("REGISTER");
    expect(recommendedAction("OPENING")).toBeNull();
  });
});

describe("auctionGuidance", () => {
  it("returns full guidance for AVAILABLE", () => {
    const g = auctionGuidance("AVAILABLE", null);
    expect(g).not.toBeNull();
    expect(g!.phase).toBe("AVAILABLE");
    expect(g!.action).toBe("Open");
    expect(g!.title).toBe("Open Auction");
    expect(g!.countdown).toBeNull();
  });

  it("includes countdown when stats are present", () => {
    const g = auctionGuidance("BIDDING", { blocksUntilReveal: 50, hoursUntilReveal: 8 });
    expect(g).not.toBeNull();
    expect(g!.phase).toBe("BIDDING");
    expect(g!.action).toBe("Bid");
    expect(g!.countdown).toEqual({ label: "Reveal starts in", blocks: 50, hours: 8 });
  });

  it("returns null for REVOKED / TRANSFER / OTHER", () => {
    expect(auctionGuidance("REVOKED", null)).toBeNull();
    expect(auctionGuidance("TRANSFER", null)).toBeNull();
    expect(auctionGuidance("OTHER", null)).toBeNull();
  });

  it("handles null/undefined state as AVAILABLE", () => {
    const g = auctionGuidance(null, null);
    expect(g).not.toBeNull();
    expect(g!.phase).toBe("AVAILABLE");
  });
});

describe("hnsToDoos / doosToHns", () => {
  it("converts HNS to doos (integer)", () => {
    expect(hnsToDoos(1)).toBe(1_000_000);
    expect(hnsToDoos(0.5)).toBe(500_000);
    expect(hnsToDoos(100)).toBe(100_000_000);
  });

  it("converts doos to HNS", () => {
    expect(doosToHns(1_000_000)).toBe(1);
    expect(doosToHns(500_000)).toBe(0.5);
    expect(doosToHns(0)).toBe(0);
  });

  it("round-trips correctly", () => {
    const hns = 12.345678;
    expect(doosToHns(hnsToDoos(hns))).toBeCloseTo(hns, 5);
  });
});
