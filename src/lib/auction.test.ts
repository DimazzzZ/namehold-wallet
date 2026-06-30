import { describe, it, expect } from "vitest";
import {
  auctionPhase,
  nextTransition,
  formatCountdown,
  recommendedAction,
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
