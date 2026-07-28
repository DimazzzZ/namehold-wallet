import { describe, it, expect } from "vitest";
import { mergeActivity } from "../activity";
import type { ActionRow } from "../zod";
import type { TxDraftSummary } from "../../types";

describe("mergeActivity", () => {
  it("dedupes: ActionRow + draft with same txid → one row with fee + status", () => {
    const rows: ActionRow[] = [
      {
        txid: "aabbccdd",
        action: "send",
        name: null,
        nameHash: null,
        valueDoos: -100_000_000,
        direction: "send",
        height: 100,
        time: 1000,
        confirmed: true,
        counterparty: "hs1qrecipient",
      },
    ];
    const drafts: TxDraftSummary[] = [
      {
        id: "draft1",
        walletProfileId: "profile1",
        action: "send",
        status: "confirmed",
        summary: {
          action: "send",
          sendTotalDoos: 100_000_000,
          feeDoos: 5_000,
          changeDoos: 0,
          inputTotalDoos: 105_000_000,
          numInputs: 1,
          recipientAddress: "hs1qrecipient",
          txid: "aabbccdd",
          warnings: [],
        },
        errorMessage: null,
        txid: "aabbccdd",
        confirmationHeight: 100,
        createdAt: "2026-07-24 12:00:00",
      },
    ];
    const merged = mergeActivity(rows, drafts);
    expect(merged).toHaveLength(1);
    expect(merged[0]!).toEqual({
      key: "aabbccdd",
      txid: "aabbccdd",
      action: "send",
      name: null,
      valueDoos: -100_000_000,
      nameValueDoos: null,
      direction: "send",
      feeDoos: 5_000,
      status: "confirmed",
      confirmed: true,
      height: 100,
      sortTs: 1000,
      counterparty: "hs1qrecipient",
    });
  });

  it("receive-only: ActionRow with no draft → feeDoos null, status onchain", () => {
    const rows: ActionRow[] = [
      {
        txid: "inbound",
        action: "receive",
        name: null,
        nameHash: null,
        valueDoos: 50_000_000,
        direction: "receive",
        height: 200,
        time: 2000,
        confirmed: true,
        counterparty: null,
      },
    ];
    const merged = mergeActivity(rows, []);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.feeDoos).toBeNull();
    expect(merged[0]!.status).toBe("onchain");
  });

  it("draft-only unbroadcast: signed draft with null txid → sorted by createdAt", () => {
    const drafts: TxDraftSummary[] = [
      {
        id: "draft-signed",
        walletProfileId: "profile1",
        action: "send",
        status: "signed",
        summary: {
          action: "send",
          sendTotalDoos: 100_000_000,
          feeDoos: 5_000,
          changeDoos: 0,
          inputTotalDoos: 105_000_000,
          numInputs: 1,
          recipientAddress: "hs1qrecipient",
          txid: null,
          warnings: [],
        },
        errorMessage: null,
        txid: null,
        confirmationHeight: null,
        createdAt: "2026-07-24 13:00:00",
      },
    ];
    const merged = mergeActivity([], drafts);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.key).toBe("draft:draft-signed");
    expect(merged[0]!.status).toBe("signed");
    expect(merged[0]!.height).toBeNull();
    expect(merged[0]!.txid).toBeNull();
  });

  it("dropped draft: txid set but no matching ActionRow → renders as its own row", () => {
    const drafts: TxDraftSummary[] = [
      {
        id: "draft-dropped",
        walletProfileId: "profile1",
        action: "send",
        status: "dropped",
        summary: {
          action: "send",
          sendTotalDoos: 100_000_000,
          feeDoos: 5_000,
          changeDoos: 0,
          inputTotalDoos: 105_000_000,
          numInputs: 1,
          recipientAddress: "hs1qrecipient",
          txid: "dropped-tx",
          warnings: [],
        },
        errorMessage: null,
        txid: "dropped-tx",
        confirmationHeight: null,
        createdAt: "2026-07-24 14:00:00",
      },
    ];
    const merged = mergeActivity([], drafts);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.status).toBe("dropped");
    expect(merged[0]!.txid).toBe("dropped-tx");
  });

  it("self-homed covenant: valueDoos 0, neutral tone, fee shown", () => {
    const drafts: TxDraftSummary[] = [
      {
        id: "draft-update",
        walletProfileId: "profile1",
        action: "update",
        status: "confirmed",
        summary: {
          action: "update",
          sendTotalDoos: 222_000_000, // name's locked value
          feeDoos: 10_000,
          changeDoos: 0,
          inputTotalDoos: 232_000_000,
          numInputs: 1,
          recipientAddress: null, // self-homed: no external recipient
          txid: "update-tx",
          warnings: [],
          name: "myname",
        },
        errorMessage: null,
        txid: "update-tx",
        confirmationHeight: 150,
        createdAt: "2026-07-24 15:00:00",
      },
    ];
    const merged = mergeActivity([], drafts);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.valueDoos).toBe(0);
    expect(merged[0]!.direction).toBe("internal");
    expect(merged[0]!.feeDoos).toBe(10_000);
  });

  it("sort: mixed rows ordered newest-first by sortTs", () => {
    // Use realistic unix seconds so on-chain `time` and parsed `createdAt`
    // live in the same numeric range. 2026-07-24 timestamps:
    const t1200 = Math.floor(Date.UTC(2026, 6, 24, 12, 0, 0) / 1000);
    const t1230 = Math.floor(Date.UTC(2026, 6, 24, 12, 30, 0) / 1000);
    const t1300 = Math.floor(Date.UTC(2026, 6, 24, 13, 0, 0) / 1000);
    const rows: ActionRow[] = [
      {
        txid: "old-tx",
        action: "send",
        name: null,
        nameHash: null,
        valueDoos: -100_000_000,
        direction: "send",
        height: 50,
        time: t1200, // oldest
        confirmed: true,
        counterparty: null,
      },
      {
        txid: "recent-tx",
        action: "receive",
        name: null,
        nameHash: null,
        valueDoos: 50_000_000,
        direction: "receive",
        height: 100,
        time: t1300, // newest on-chain
        confirmed: true,
        counterparty: null,
      },
    ];
    const drafts: TxDraftSummary[] = [
      {
        id: "draft-mid",
        walletProfileId: "profile1",
        action: "send",
        status: "signed",
        summary: {
          action: "send",
          sendTotalDoos: 100_000_000,
          feeDoos: 5_000,
          changeDoos: 0,
          inputTotalDoos: 105_000_000,
          numInputs: 1,
          recipientAddress: "hs1q",
          txid: null,
          warnings: [],
        },
        errorMessage: null,
        txid: null,
        confirmationHeight: null,
        createdAt: "2026-07-24 12:30:00", // middle timestamp
      },
    ];
    const merged = mergeActivity(rows, drafts);
    expect(merged).toHaveLength(3);
    expect(merged[0]!.sortTs).toBe(t1300); // recent-tx (newest)
    expect(merged[1]!.sortTs).toBe(t1230); // draft-mid (parsed createdAt)
    expect(merged[2]!.sortTs).toBe(t1200); // old-tx (oldest)
  });

  it("sign convention: draft-only send yields negative valueDoos", () => {
    const drafts: TxDraftSummary[] = [
      {
        id: "draft-send",
        walletProfileId: "profile1",
        action: "send",
        status: "broadcasted",
        summary: {
          action: "send",
          sendTotalDoos: 100_000_000,
          feeDoos: 5_000,
          changeDoos: 0,
          inputTotalDoos: 105_000_000,
          numInputs: 1,
          recipientAddress: "hs1qrecipient",
          txid: "send-tx",
          warnings: [],
        },
        errorMessage: null,
        txid: "send-tx",
        confirmationHeight: null,
        createdAt: "2026-07-24 12:00:00",
      },
    ];
    const merged = mergeActivity([], drafts);
    expect(merged[0]!.valueDoos).toBe(-100_000_000); // negative (outflow)
  });
});
