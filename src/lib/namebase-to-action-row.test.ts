import { describe, it, expect } from "vitest";
import {
  namebaseEventToActionRow,
  verbToAction,
} from "./namebase-to-action-row";
import type { NamebaseHistoryRow } from "../queries/namebase";

function makeRow(overrides: Partial<NamebaseHistoryRow> = {}): NamebaseHistoryRow {
  return {
    id: 1,
    createdAt: "2026-01-17T12:37:54.492Z",
    type: "auctions:place-bid:4",
    family: "auctions",
    verb: "place-bid",
    name: "diver",
    feeDoos: null,
    bidDoos: null,
    stakeDoos: null,
    usdCents: null,
    hnsDoos: null,
    auctionId: null,
    bidId: null,
    saleId: null,
    dataJson: "",
    importedAt: "2026-01-26T00:00:00Z",
    ...overrides,
  };
}

describe("namebaseEventToActionRow", () => {
  it("converts a place-bid to a bid action with internal direction", () => {
    const row = makeRow({
      id: 188679284,
      feeDoos: 1000283,
      bidDoos: 123000000,
      stakeDoos: 2469000000,
    });
    const result = namebaseEventToActionRow(row);
    expect(result.txid).toBe("nb:188679284");
    expect(result.action).toBe("bid");
    expect(result.name).toBe("diver");
    expect(result.valueDoos).toBe(0); // name-covenant: no net spend
    expect(result.direction).toBe("internal");
    expect(result.confirmed).toBe(true);
    expect(result.height).toBeNull();
    expect(result.source).toBe("namebase");
    expect(result.feeDoos).toBe(1000283);
    expect(result.bidDoos).toBe(123000000);
  });

  it("converts a deposit to a receive action with HNS value", () => {
    const row = makeRow({
      id: 456,
      createdAt: "2020-09-22T10:03:36.467Z",
      type: "wallet:deposit:1",
      family: "wallet",
      verb: "deposit",
      name: null,
      hnsDoos: 1097674,
    });
    const result = namebaseEventToActionRow(row);
    expect(result.action).toBe("receive");
    expect(result.valueDoos).toBe(1097674);
    expect(result.direction).toBe("receive");
    expect(result.hnsDoos).toBe(1097674);
  });

  it("converts a confirm-transfer to a transfer action", () => {
    const row = makeRow({
      id: 789,
      type: "subdomains:confirm-transfer:2",
      family: "subdomains",
      verb: "confirm-transfer",
      name: "shot",
      usdCents: 2900,
      hnsDoos: 4832721250,
    });
    const result = namebaseEventToActionRow(row);
    expect(result.action).toBe("transfer");
    expect(result.valueDoos).toBe(0); // subdomain sale: locked value re-homes
    expect(result.direction).toBe("internal");
    expect(result.usdCents).toBe(2900);
  });

  it("converts a negative admin-gift to a send", () => {
    const row = makeRow({
      id: 33287041,
      type: "misc:admin-gift:1",
      family: "misc",
      verb: "admin-gift",
      name: null,
      hnsDoos: -100000000,
    });
    const result = namebaseEventToActionRow(row);
    expect(result.action).toBe("receive"); // verb maps to "receive"
    expect(result.valueDoos).toBe(-100000000);
    expect(result.direction).toBe("send"); // negative HNS = outflow
  });

  it("parses createdAt into Unix seconds", () => {
    const row = makeRow({ createdAt: "2026-01-17T12:37:54.492Z" });
    const result = namebaseEventToActionRow(row);
    expect(result.time).toBe(Math.floor(new Date("2026-01-17T12:37:54.492Z").getTime() / 1000));
  });

  it("synthetic txid never collides with a real 64-hex txid", () => {
    const result = namebaseEventToActionRow(makeRow({ id: 999 }));
    expect(result.txid).toBe("nb:999");
    expect(result.txid).not.toMatch(/^[0-9a-f]{64}$/);
  });
});

describe("verbToAction", () => {
  it("maps all known verbs correctly", () => {
    expect(verbToAction("place-bid")).toBe("bid");
    expect(verbToAction("reveal-bid")).toBe("reveal");
    expect(verbToAction("redeem-bid")).toBe("redeem");
    expect(verbToAction("register-bid")).toBe("register");
    expect(verbToAction("update-domain")).toBe("update");
    expect(verbToAction("confirm-transfer")).toBe("transfer");
    expect(verbToAction("initialize-transfer")).toBe("transfer");
    expect(verbToAction("buy-now")).toBe("transfer");
    expect(verbToAction("deposit")).toBe("receive");
    expect(verbToAction("charge-renewal-fee")).toBe("renew");
  });

  it("returns 'other' for unknown verbs", () => {
    expect(verbToAction("something-new")).toBe("other");
  });
});
