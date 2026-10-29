import { describe, it, expect } from "vitest";
import { approxBlocksDate, countdownForNetwork, nameStatusLine, nameTimeline } from "../nameStatus";
import { namePath, parseNameTab } from "../nameRoute";
import { makeCapabilities } from "../../test/fixtures/capabilities";
import type { AuctionTaskState, HsdName } from "../../types";

const NOW = Date.UTC(2026, 9, 28);
const info = (over: Partial<HsdName> = {}): HsdName => ({
  name: "x",
  state: "CLOSED",
  height: 811204,
  renewal: 890204,
  owner: { hash: "t", index: 0 },
  stats: { blocksUntilExpire: 98120 },
  highest: 18_250_000,
  ...over,
});
const status = (taskState: AuctionTaskState, network = "mainnet", over = {}) =>
  nameStatusLine({
    caps: makeCapabilities({ name: "x", taskState, ...over }),
    info: info(),
    countdown: { label: "Reveal ends in", blocks: 5, hours: 0.8 },
    network,
    nowMs: NOW,
  });

describe("nameStatusLine", () => {
  it.each([
    ["availableToOpen", "Available"],
    ["waitingForBidding", "Auction opening"],
    ["readyToBid", "Bidding is open"],
    ["readyToReveal", "Reveal your bid"],
    ["revealBroadcastPending", "Reveal sent"],
    ["revealDoneWaitingForClose", "Revealed"],
    ["wonNeedsRegister", "You won"],
    ["lostNeedsRedeem", "You lost"],
    ["lostUnrevealed", "Lost — not revealed in time"],
    ["closedResultUnknown", "Auction closed"],
    ["transferPendingFinalize", "Transfer in progress"],
    ["ownedNoUrgentAction", "Owned"],
    ["expiringSoon", "Expiring"],
    ["unavailableOther", "Registered by someone else"],
  ] as [AuctionTaskState, string][])("%s → %s", (t, s) => {
    expect(status(t).status).toBe(s);
  });

  it("dates the expiry on mainnet and counts blocks on regtest", () => {
    const main = status("ownedNoUrgentAction");
    expect(main.sub).toMatch(/^renew by [A-Z][a-z]{2} 2028$/);
    expect(main.note).toBe("(98,120 blocks)");
    const reg = status("ownedNoUrgentAction", "regtest");
    expect(reg).toMatchObject({ sub: "renew within 98,120 blocks", note: null });
    expect(status("expiringSoon", "regtest").sub).toBe(
      "renew within 98,120 blocks or the name is lost",
    );
    expect(status("unavailableOther", "regtest").sub).toBe("expires in 98,120 blocks");
  });

  it("shows the countdown with hours on mainnet and blocks only on regtest", () => {
    expect(status("readyToReveal").sub).toBe(
      "reveal ends in 5 blocks (~48m), or the lockup is lost",
    );
    expect(status("readyToReveal", "regtest").sub).toBe(
      "reveal ends in 5 blocks, or the lockup is lost",
    );
  });

  it("overrides: pending, loading, error, no caps", () => {
    expect(status("readyToReveal", "mainnet", { pendingBroadcastAction: "reveal" }).status).toBe(
      "Reveal sent",
    );
    expect(
      nameStatusLine({ caps: null, info: null, countdown: null, network: "mainnet", loading: true })
        .status,
    ).toBe("Checking…");
    expect(
      nameStatusLine({ caps: null, info: null, countdown: null, network: "mainnet", error: true })
        .status,
    ).toBe("Couldn't load this name");
    expect(
      nameStatusLine({
        caps: null,
        info: info({ state: "BIDDING" }),
        countdown: null,
        network: "mainnet",
      }).status,
    ).toBe("Bidding");
  });
});

describe("approxBlocksDate / countdownForNetwork", () => {
  it("is null on regtest and dates otherwise", () => {
    expect(approxBlocksDate(100, "regtest", NOW)).toBeNull();
    expect(approxBlocksDate(30, "mainnet", NOW)).toEqual({ short: "Oct 28", rel: "~5h" });
    expect(approxBlocksDate(288 * 2, "mainnet", NOW)!.rel).toBe("~4 days");
    expect(approxBlocksDate(98120, "mainnet", NOW)!.rel).toBe("~22 months");
    expect(approxBlocksDate(120000, "mainnet", NOW)!.rel).toBe("~2.3 years");
  });
  it("drops hours on regtest", () => {
    const c = { label: "x", blocks: 5, hours: 1 };
    expect(countdownForNetwork(c, "regtest").hours).toBeNull();
    expect(countdownForNetwork(c, "testnet").hours).toBe(1);
  });
});

describe("nameTimeline", () => {
  const steps = (taskState: AuctionTaskState, phase: string | null, over = {}) =>
    nameTimeline({
      caps: makeCapabilities({ name: "x", taskState, phase: phase as never, ...over }),
      info: info({ state: phase }),
      bidCount: "3 bids",
      countdown: { label: "Reveal ends in", blocks: 5, hours: 0.8 },
      network: "mainnet",
      nowMs: NOW,
    }).map((s) => `${s.label}:${s.kind}`);

  it.each([
    [
      "availableToOpen",
      "AVAILABLE",
      [
        "Open:current",
        "Bidding:next",
        "Reveal:next",
        "Result:next",
        "Registered:next",
        "Renew:next",
      ],
    ],
    [
      "readyToBid",
      "BIDDING",
      [
        "Opened:done",
        "Bidding:current",
        "Reveal:next",
        "Result:next",
        "Registered:next",
        "Renew:next",
      ],
    ],
    [
      "readyToReveal",
      "REVEAL",
      [
        "Opened:done",
        "Bidding:done",
        "Reveal:current",
        "Result:next",
        "Registered:next",
        "Renew:next",
      ],
    ],
    [
      "wonNeedsRegister",
      "CLOSED",
      [
        "Opened:done",
        "Bidding:done",
        "Reveal:done",
        "Won:current",
        "Registered:next",
        "Renew:next",
      ],
    ],
    [
      "lostNeedsRedeem",
      "CLOSED",
      ["Opened:done", "Bidding:done", "Reveal:done", "Lost:lost", "Registered:na", "Renew:na"],
    ],
    [
      "unavailableOther",
      "REVOKED",
      ["Opened:done", "Bidding:done", "Reveal:done", "Won:done", "Revoked:lost", "Renew:na"],
    ],
    [
      "unavailableOther",
      "CLOSED",
      [
        "Opened:done",
        "Bidding:done",
        "Reveal:done",
        "Won:done",
        "Registered:current",
        "Expires:next",
      ],
    ],
  ] as [AuctionTaskState, string, string[]][])("%s / %s", (t, p, expected) => {
    expect(steps(t, p)).toEqual(expected);
  });

  it("owned: Registered current, then Expires or Renew due", () => {
    expect(steps("ownedNoUrgentAction", "CLOSED", { ownsName: true }).slice(4)).toEqual([
      "Registered:current",
      "Expires:next",
    ]);
    expect(steps("expiringSoon", "CLOSED", { ownsName: true }).slice(4)).toEqual([
      "Registered:current",
      "Renew due:next",
    ]);
    expect(
      steps("transferPendingFinalize", "TRANSFER", { ownsName: true, transferPending: true }).slice(
        4,
      ),
    ).toEqual(["Transfer:current", "Finalize:next"]);
  });
});

describe("nameRoute", () => {
  it("encodes the raw name and adds ?tab except for overview", () => {
    expect(namePath("xn--mnchen-3ya")).toBe("/names/xn--mnchen-3ya");
    expect(namePath("a b", "dns")).toBe("/names/a%20b?tab=dns");
    expect(namePath("x", "overview")).toBe("/names/x");
  });
  it("falls back to overview", () => {
    expect(parseNameTab(null)).toBe("overview");
    expect(parseNameTab("nope")).toBe("overview");
    expect(parseNameTab("sign")).toBe("sign");
  });
});
