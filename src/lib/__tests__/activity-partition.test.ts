import { describe, it, expect } from "vitest";
import { isToFinish, isWaiting, partitionActivity, type MergedRow } from "../activity";
import { formatDateShort } from "../utils";
import {
  finishSentence,
  nameLabel,
  statusBadge,
  waitingSentence,
} from "../../components/activity/activityText";

const TS = Math.floor(Date.UTC(2026, 6, 30, 14, 9) / 1000);

const r = (over: Partial<MergedRow>): MergedRow => ({
  key: over.txid ?? `draft:${over.draftId ?? "x"}`,
  txid: null,
  draftId: "d1",
  action: "send",
  covenantAction: "send",
  name: null,
  nameList: null,
  valueDoos: -75_000_000,
  nameValueDoos: null,
  direction: "send",
  feeDoos: 210,
  status: "draft",
  statusNote: null,
  lostReason: null,
  confirmed: false,
  height: null,
  sortTs: TS,
  counterparty: null,
  speedsUp: null,
  spedUpBy: null,
  ...over,
});

describe("isToFinish / isWaiting / partitionActivity", () => {
  it.each([
    ["draft", "toFinish"],
    ["signed", "toFinish"],
    ["failed", "toFinish"],
    ["dropped", "toFinish"],
    ["broadcasted", "waiting"],
    ["broadcast_pending", "waiting"],
    ["confirmed", "history"],
  ] as const)("a %s draft goes to %s", (status, list) => {
    const row = r({ status, confirmed: status === "confirmed" });
    const parts = partitionActivity([row]);
    expect(parts[list]).toEqual([row]);
  });

  it("a lost purchase still dropped is to finish; one lost after it paid is history", () => {
    const dropped = r({ action: "shakedex_purchase", status: "dropped", lostReason: "x" });
    const paid = r({
      action: "shakedex_purchase",
      status: "confirmed",
      confirmed: true,
      lostReason: "x",
    });
    expect(isToFinish(dropped)).toBe(true);
    expect(isToFinish(paid)).toBe(false);
    expect(isWaiting(paid)).toBe(false);
  });

  it("a broadcasted draft the chain already mined is history", () => {
    const row = r({ status: "broadcasted", confirmed: true, txid: "t" });
    expect(partitionActivity([row]).history).toEqual([row]);
  });

  it("an unconfirmed chain-only row (a receive in the mempool) is waiting", () => {
    const row = r({ draftId: null, status: "onchain", txid: "t", direction: "receive" });
    expect(isWaiting(row)).toBe(true);
    expect(isToFinish(row)).toBe(false);
  });

  it("keeps history in input order", () => {
    const a = r({ draftId: null, status: "onchain", confirmed: true, txid: "a" });
    const b = r({ draftId: null, status: "onchain", confirmed: true, txid: "b" });
    expect(partitionActivity([a, b]).history).toEqual([a, b]);
  });
});

describe("statusBadge", () => {
  it("says Confirmed for a broadcasted draft the chain has mined", () => {
    expect(statusBadge("broadcasted", { confirmed: true, height: 5 }).label).toBe("Confirmed");
    expect(statusBadge("broadcasted", { confirmed: false, height: null }).label).toBe("Pending");
  });
});

describe("activityText sentences", () => {
  it("names a batch by its first name, never its synthetic label", () => {
    expect(nameLabel(r({ name: "js + 1 more", nameList: ["js", "c", "d"] }))).toBe(".js + 2 more");
    expect(nameLabel(r({ name: "foo" }))).toBe(".foo");
    expect(nameLabel(r({}))).toBe("");
  });

  it("builds the To finish sentence per status", () => {
    expect(finishSentence(r({ action: "renew", name: "wallet-guy" }))).toEqual({
      lead: "Renew ",
      name: ".wallet-guy",
      tail: " was built but not sent.",
      detail:
        "Draft from Jul 30, 14:09 · fee 0.000210 HNS. Its coins stay reserved until you send or discard it.",
      detailTone: "muted",
    });
    expect(finishSentence(r({ status: "signed" })).detail).toBe(
      "Built Jul 30, 14:09. Broadcast it, or discard it to free its coins.",
    );
    expect(finishSentence(r({ status: "failed", statusNote: "Bad fee." })).detail).toBe(
      "Jul 30, 14:09. Nothing left this wallet. Bad fee.",
    );
    expect(finishSentence(r({ action: "transfer", name: "h", status: "dropped" })).detail).toBe(
      "Sent Jul 30, 14:09. The network dropped it; nothing happened on-chain.",
    );
  });

  it("trims the lead when the row has no name", () => {
    const s = finishSentence(r({ feeDoos: null }));
    expect(s.lead).toBe("Send 75 HNS");
    expect(s.detail).not.toContain("fee");
  });

  it("words a lost purchase, dropping the amount when it is zero", () => {
    const lost = finishSentence(
      r({
        action: "shakedex_purchase",
        name: "pixelart",
        status: "dropped",
        valueDoos: -410_200_000,
        lostReason: "Someone else bought it.",
      }),
    );
    expect(lost.lead + lost.name + lost.tail).toBe(
      "Buying .pixelart for 410.2 HNS didn't go through.",
    );
    expect(lost.detail).toBe("Jul 30, 14:09. Someone else bought it.");
    expect(lost.detailTone).toBe("error");
    const zero = finishSentence(
      r({ action: "shakedex_purchase", name: "p", valueDoos: 0, lostReason: "x" }),
    );
    expect(zero.tail).toBe(" didn't go through.");
  });

  it("builds the waiting sentence", () => {
    expect(waitingSentence(r({ status: "broadcasted" }))).toEqual({
      lead: "Send 75 HNS",
      name: "",
      tail: "",
    });
    expect(
      waitingSentence(
        r({
          action: "bid",
          covenantAction: "bid",
          name: "nightjar",
          valueDoos: 0,
          nameValueDoos: 12_000_000,
        }),
      ),
    ).toEqual({ lead: "Bid on ", name: ".nightjar", tail: " · 12 HNS lockup" });
  });
});

describe("formatDateShort", () => {
  it("formats UTC, with or without the year", () => {
    expect(formatDateShort("2026-07-30T14:09:00Z")).toBe("Jul 30, 2026 · 14:09");
    expect(formatDateShort("2026-07-30 14:09:00", { year: false })).toBe("Jul 30, 14:09");
    expect(formatDateShort(null)).toBe("—");
    expect(formatDateShort("")).toBe("—");
  });
});
