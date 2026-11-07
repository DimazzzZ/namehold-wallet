import { describe, it, expect } from "vitest";
import {
  attentionText,
  canCancel,
  commitSentence,
  exchangeTimeText,
  myCommitDeadline,
  needsMyCommit,
  tradeStatusText,
} from "../exchangeStatus";
import { EVERY_STATUS, T0, makeTimeline, makeTrade } from "../../test/fixtures/exchange";
import type { NeedsAttentionReason, TradeRole, TradeStatus } from "../../types/exchange";

const U64_MAX = 18_446_744_073_709_551_615;
const H = 3600;

/** Headline per status (keyed by kind, chain and reason) and role. */
const HEADLINES: Record<string, Record<TradeRole, string>> = {
  awaitingBuyerCommit: {
    buyer: "Confirm to start the trade",
    seller: "Waiting for the buyer to confirm",
  },
  awaitingBtcLock: { buyer: "Locking your BTC", seller: "Waiting for the buyer's BTC" },
  awaitingSellerCommit: {
    buyer: "Waiting for the seller to confirm",
    seller: "Confirm to lock your HNS",
  },
  awaitingHnsLock: { buyer: "Waiting for the seller's HNS", seller: "Locking your HNS" },
  revealing: { buyer: "Claiming your HNS", seller: "The buyer is claiming the HNS" },
  awaitingReveal: { buyer: "Ready to claim your HNS", seller: "Waiting for the buyer to claim" },
  claiming: { buyer: "The seller is claiming the BTC", seller: "Claiming your BTC" },
  completed: { buyer: "Trade complete", seller: "Trade complete" },
  "awaitingRefund/hns": {
    buyer: "The seller is waiting to take their HNS back",
    seller: "Waiting to take your HNS back",
  },
  "awaitingRefund/btc": {
    buyer: "Waiting to take your BTC back",
    seller: "The buyer is waiting to take their BTC back",
  },
  "refunding/hns": {
    buyer: "The seller is taking their HNS back",
    seller: "Taking your HNS back",
  },
  "refunding/btc": {
    buyer: "Taking your BTC back",
    seller: "The buyer is taking their BTC back",
  },
  "refunded/hns": { buyer: "The seller took their HNS back", seller: "Your HNS is back" },
  "refunded/btc": { buyer: "Your BTC is back", seller: "The buyer took their BTC back" },
  expired: { buyer: "Trade expired", seller: "Trade expired" },
  "cannotCheck/hns": { buyer: "Can't check the HNS chain", seller: "Can't check the HNS chain" },
  "cannotCheck/btc": { buyer: "Can't check Bitcoin", seller: "Can't check Bitcoin" },
  "needsAttention/sellerLockedWrongAmount": {
    buyer: "The seller locked the wrong amount",
    seller: "The seller locked the wrong amount",
  },
  "needsAttention/buyerLockedWrongAmount": {
    buyer: "The buyer locked the wrong amount",
    seller: "The buyer locked the wrong amount",
  },
  "needsAttention/btcRefundedAfterReveal": {
    buyer: "The BTC went back after the secret was public",
    seller: "The BTC went back after the secret was public",
  },
  "needsAttention/hnsRefundedAfterReveal": {
    buyer: "The HNS went back after the secret was public",
    seller: "The HNS went back after the secret was public",
  },
  "needsAttention/missingPresigned": {
    buyer: "A signed transaction is missing",
    seller: "A signed transaction is missing",
  },
};

function keyOf(s: TradeStatus): string {
  if (s.kind === "needsAttention") return `needsAttention/${s.reason}`;
  if ("chain" in s) return `${s.kind}/${s.chain}`;
  return s.kind;
}

describe("exchangeTimeText", () => {
  it("formats Unix seconds in UTC", () => {
    expect(exchangeTimeText(T0)).toBe("Nov 7, 2026, 12:00 UTC");
  });

  it("returns null for values no Date can hold, without throwing (u64 safety)", () => {
    expect(exchangeTimeText(U64_MAX)).toBeNull();
    expect(exchangeTimeText(Number.MAX_SAFE_INTEGER)).toBeNull();
    expect(exchangeTimeText(Number.NaN)).toBeNull();
    expect(exchangeTimeText(Number.POSITIVE_INFINITY)).toBeNull();
    expect(exchangeTimeText(-1)).toBeNull();
  });
});

describe("needsMyCommit", () => {
  const cases: [TradeStatus["kind"], TradeRole, boolean][] = [
    ["awaitingBuyerCommit", "buyer", true],
    ["awaitingBuyerCommit", "seller", false],
    ["awaitingSellerCommit", "seller", true],
    ["awaitingSellerCommit", "buyer", false],
  ];
  it.each(cases)("%s as %s → %s", (kind, role, expected) => {
    const status = (
      kind === "awaitingBuyerCommit"
        ? { kind, deadline: T0 + H }
        : { kind: "awaitingSellerCommit", deadline: T0 + 4 * H }
    ) as TradeStatus;
    expect(needsMyCommit({ status, role })).toBe(expected);
  });

  it("is false for every other status, for both roles", () => {
    for (const status of EVERY_STATUS) {
      if (status.kind === "awaitingBuyerCommit" || status.kind === "awaitingSellerCommit") continue;
      for (const role of ["buyer", "seller"] as const) {
        expect(needsMyCommit({ status, role }), `${keyOf(status)} as ${role}`).toBe(false);
      }
    }
  });

  it("gives the deadline of the Commit it waits for", () => {
    expect(
      myCommitDeadline({ status: { kind: "awaitingSellerCommit", deadline: 42 }, role: "seller" }),
    ).toBe(42);
    expect(
      myCommitDeadline({ status: { kind: "awaitingSellerCommit", deadline: 42 }, role: "buyer" }),
    ).toBeNull();
  });
});

describe("canCancel", () => {
  it("is offered while negotiating, whatever the status", () => {
    expect(canCancel(makeTrade({ negotiating: true, status: { kind: "completed" } }))).toBe(true);
  });

  it("is offered to the buyer only before the buyer's Commit", () => {
    for (const status of EVERY_STATUS) {
      expect(canCancel(makeTrade({ role: "buyer", status })), keyOf(status)).toBe(
        status.kind === "awaitingBuyerCommit",
      );
    }
  });

  it("is offered to the seller until the seller's Commit", () => {
    const before = new Set(["awaitingBuyerCommit", "awaitingBtcLock", "awaitingSellerCommit"]);
    for (const status of EVERY_STATUS) {
      expect(canCancel(makeTrade({ role: "seller", status })), keyOf(status)).toBe(
        before.has(status.kind),
      );
    }
  });

  it("is not offered on a trade that already ended, even one never agreed", () => {
    // An own cancel ends a negotiating trade as expired, with no decline.
    expect(canCancel(makeTrade({ negotiating: true, status: { kind: "expired" } }))).toBe(false);
  });

  it("is not offered on a declined trade", () => {
    expect(canCancel(makeTrade({ declined: "price moved", negotiating: true }))).toBe(false);
  });
});

describe("tradeStatusText", () => {
  it("has a headline for every variant in the fixture list", () => {
    expect(new Set(EVERY_STATUS.map(keyOf))).toEqual(new Set(Object.keys(HEADLINES)));
  });

  const rows = EVERY_STATUS.flatMap((status) =>
    (["buyer", "seller"] as const).map((role) => [keyOf(status), role, status] as const),
  );
  it.each(rows)("%s as %s", (key, role, status) => {
    const t = tradeStatusText(makeTrade({ role, status }));
    expect(t.headline).toBe(HEADLINES[key]![role]);
    expect(t.sentence.length).toBeGreaterThan(20);
    expect(t.sentence.endsWith(".")).toBe(true);
  });

  it("the buyer's Commit signs everything at once and the BTC comes back after btcRefundTime", () => {
    const t = tradeStatusText(makeTrade({ role: "buyer" }));
    expect(t.tone).toBe("action");
    expect(t.sentence).toContain("signs every transaction of your side at once");
    expect(t.sentence).toContain("before Nov 7, 2026, 13:00 UTC");
    expect(t.sentence).toContain(
      "If the seller never locks the HNS, your BTC comes back to you automatically after Nov 8, 2026, 12:00 UTC.",
    );
  });

  it("the seller's Commit signs everything at once and the HNS comes back after hnsRefundTime", () => {
    const t = tradeStatusText(
      makeTrade({ role: "seller", status: { kind: "awaitingSellerCommit", deadline: T0 + 4 * H } }),
    );
    expect(t.tone).toBe("action");
    expect(t.sentence).toContain("signs every transaction of your side at once");
    expect(t.sentence).toContain(
      "If the buyer never claims the HNS, it comes back to you automatically after Nov 8, 2026, 00:00 UTC.",
    );
  });

  it("names the refund time in words when the timeline is unknown or cannot be a Date", () => {
    expect(commitSentence("buyer", T0, null)).toContain("after the BTC refund time.");
    const t = tradeStatusText(
      makeTrade({
        role: "seller",
        status: { kind: "awaitingSellerCommit", deadline: U64_MAX },
        timeline: makeTimeline({ hnsRefundTime: U64_MAX }),
      }),
    );
    expect(t.sentence).toContain("before the deadline");
    expect(t.sentence).toContain("after the HNS refund time.");
  });

  it("shows confirmations as counted", () => {
    const t = tradeStatusText(
      makeTrade({
        role: "seller",
        status: { kind: "awaitingBtcLock", confirmations: 1, needed: 3 },
      }),
    );
    expect(t.sentence).toContain("1 of 3 confirmations");
  });

  it("gives each needsAttention reason its own sentence, for each role", () => {
    const reasons: NeedsAttentionReason[] = [
      "sellerLockedWrongAmount",
      "buyerLockedWrongAmount",
      "btcRefundedAfterReveal",
      "hnsRefundedAfterReveal",
      "missingPresigned",
    ];
    for (const role of ["buyer", "seller"] as const) {
      const sentences = reasons.map(
        (r) => attentionText(r, "hns", role, { timeline: makeTimeline() }).sentence,
      );
      expect(new Set(sentences).size).toBe(reasons.length);
    }
    expect(
      attentionText("missingPresigned", "btc", "buyer", { timeline: null }).sentence,
    ).toContain("A Bitcoin transaction");
    expect(
      attentionText("missingPresigned", "hns", "buyer", { timeline: null }).sentence,
    ).toContain("A Handshake transaction");
  });

  it("describes a status this build does not know instead of throwing", () => {
    const status = { kind: "somethingNew" } as unknown as TradeStatus;
    expect(tradeStatusText(makeTrade({ status })).headline).toBe("Unknown status");
  });
});
