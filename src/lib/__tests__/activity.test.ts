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
        nameValueDoos: null,
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
        purchaseLostReason: null,
        createdAt: "2026-07-24 12:00:00",
      },
    ];
    const merged = mergeActivity(rows, drafts);
    expect(merged).toHaveLength(1);
    expect(merged[0]!).toEqual({
      key: "aabbccdd",
      txid: "aabbccdd",
      draftId: "draft1",
      action: "send",
      covenantAction: "send",
      name: null,
      nameList: null,
      valueDoos: -100_000_000,
      nameValueDoos: null,
      direction: "send",
      feeDoos: 5_000,
      status: "confirmed",
      statusNote: null,
      lostReason: null,
      confirmed: true,
      height: 100,
      sortTs: 1000,
      counterparty: "hs1qrecipient",
      speedsUp: null,
      spedUpBy: null,
    });
  });

  // A Speed up child's draft as `build_speed_up_draft` stores it
  // (`speed_up.rs::SpeedUpSummary`), trimmed to what the merge reads.
  const child = (status: TxDraftSummary["status"]): TxDraftSummary => ({
    id: "child1",
    walletProfileId: "p1",
    action: "speed_up",
    status,
    summary: {
      action: "speed_up",
      sendTotalDoos: 0,
      feeDoos: 2_340,
      changeDoos: 997_660,
      inputTotalDoos: 1_000_000,
      numInputs: 1,
      recipientAddress: null,
      txid: "c".repeat(64),
      warnings: [],
      parentTxid: "p".repeat(64),
    },
    errorMessage: null,
    txid: status === "draft" ? null : "c".repeat(64),
    confirmationHeight: null,
    purchaseLostReason: null,
    createdAt: "2026-10-21 12:00:00",
  });
  const parentRow: ActionRow = {
    txid: "p".repeat(64),
    action: "send",
    name: null,
    nameHash: null,
    valueDoos: -100_000,
    nameValueDoos: null,
    direction: "send",
    height: null,
    time: 1000,
    confirmed: false,
    counterparty: null,
  };

  it("links a Speed up child and the transaction it speeds up (R16)", () => {
    const merged = mergeActivity([parentRow], [child("broadcasted")]);
    const parent = merged.find((r) => r.txid === parentRow.txid)!;
    const kid = merged.find((r) => r.draftId === "child1")!;
    expect(parent.spedUpBy).toBe("c".repeat(64));
    expect(parent.speedsUp).toBeNull();
    expect(kid.speedsUp).toBe("p".repeat(64));
    // An unsent child is named by its build-time txid.
    const unsent = mergeActivity([parentRow], [child("draft")]);
    expect(unsent.find((r) => r.txid === parentRow.txid)!.spedUpBy).toBe("c".repeat(64));
  });

  it.each(["dropped", "failed"] as const)("a %s child speeds nothing up", (status) => {
    const merged = mergeActivity([parentRow], [child(status)]);
    expect(merged.find((r) => r.txid === parentRow.txid)!.spedUpBy).toBeNull();
  });

  it("a Shakedex draft keeps its own action once its tx is seen, not the covenant's", () => {
    const row = (txid: string): ActionRow => ({
      txid,
      action: "transfer",
      name: null,
      nameHash: null,
      valueDoos: 0,
      nameValueDoos: null,
      direction: "internal",
      height: null,
      time: null,
      confirmed: false,
      counterparty: null,
    });
    const draft = (id: string, txid: string, action: string): TxDraftSummary => ({
      id,
      walletProfileId: "profile1",
      action,
      status: "broadcasted",
      summary: null,
      errorMessage: null,
      txid,
      confirmationHeight: null,
      purchaseLostReason: null,
      createdAt: "2026-10-14 12:00:00",
    });
    const merged = mergeActivity(
      [row("aa"), row("bb")],
      [draft("d1", "aa", "shakedex_purchase"), draft("d2", "bb", "transfer")],
    );
    const actionOf = (txid: string) => merged.find((m) => m.txid === txid)?.action;
    expect(actionOf("aa")).toBe("shakedex_purchase");
    expect(actionOf("bb")).toBe("transfer");
    // The chain's label stays on the row for what the Amount cell decides.
    const covenantOf = (txid: string) => merged.find((m) => m.txid === txid)?.covenantAction;
    expect(covenantOf("aa")).toBe("transfer");
    expect(covenantOf("bb")).toBe("transfer");
  });

  it("a Shakedex draft not seen on chain yet carries the covenant it builds", () => {
    const draft = (id: string, action: string): TxDraftSummary => ({
      id,
      walletProfileId: "profile1",
      action,
      status: "signed",
      summary: null,
      errorMessage: null,
      txid: null,
      confirmationHeight: null,
      purchaseLostReason: null,
      createdAt: "2026-10-14 12:00:00",
    });
    const merged = mergeActivity(
      [],
      [
        draft("d1", "shakedex_lock"),
        draft("d2", "shakedex_lock_finalize"),
        draft("d3", "shakedex_cancel"),
        draft("d4", "shakedex_cancel_finalize"),
        draft("d5", "shakedex_purchase"),
        draft("d6", "shakedex_purchase_finalize"),
        draft("d7", "update"),
        draft("d8", "shakedex_unknown"),
      ],
    );
    const covenantOf = (id: string) => merged.find((m) => m.draftId === id)?.covenantAction;
    expect(["d1", "d2", "d3", "d4", "d5", "d6"].map(covenantOf)).toEqual([
      "transfer",
      "finalize",
      "transfer",
      "finalize",
      "transfer",
      "finalize",
    ]);
    expect(covenantOf("d7")).toBe("update");
    expect(covenantOf("d8")).toBeNull();
  });

  it("receive-only: ActionRow with no draft → feeDoos null, status onchain", () => {
    const rows: ActionRow[] = [
      {
        txid: "inbound",
        action: "receive",
        name: null,
        nameHash: null,
        valueDoos: 50_000_000,
        nameValueDoos: null,
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
        purchaseLostReason: null,
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

  it("statusNote carries a dropped or failed draft's message, and nothing else's", () => {
    const base: TxDraftSummary = {
      id: "d",
      walletProfileId: "profile1",
      action: "shakedex_purchase",
      status: "dropped",
      summary: null as never,
      errorMessage:
        "someone else bought the name first, or the seller cancelled the listing — nothing was paid",
      txid: null,
      confirmationHeight: null,
      purchaseLostReason: null,
      createdAt: "2026-07-24 14:00:00",
    };
    const note = (over: Partial<TxDraftSummary>) =>
      mergeActivity([], [{ ...base, ...over }])[0]!.statusNote;
    expect(note({})).toBe(
      "someone else bought the name first, or the seller cancelled the listing — nothing was paid",
    );
    expect(note({ status: "failed", errorMessage: "refused" })).toBe("refused");
    expect(note({ status: "broadcasted" })).toBeNull();
    expect(note({ errorMessage: null })).toBeNull();
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
        purchaseLostReason: null,
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
        purchaseLostReason: null,
        createdAt: "2026-07-24 15:00:00",
      },
    ];
    const merged = mergeActivity([], drafts);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.valueDoos).toBe(0);
    expect(merged[0]!.direction).toBe("internal");
    expect(merged[0]!.feeDoos).toBe(10_000);
  });

  it("onchain covenant with no draft: nameValueDoos comes from the backend row", () => {
    const rows: ActionRow[] = [
      {
        txid: "reveal-tx",
        action: "reveal",
        name: null,
        nameHash: "deadbeef",
        valueDoos: 0,
        nameValueDoos: 5_000_000, // covenant output value from the backend
        direction: "internal",
        height: 300,
        time: 3000,
        confirmed: true,
        counterparty: null,
      },
    ];
    const merged = mergeActivity(rows, []);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.valueDoos).toBe(0);
    expect(merged[0]!.nameValueDoos).toBe(5_000_000);
  });

  it("draft fallback: backend nameValueDoos null → uses matched draft send total", () => {
    const rows: ActionRow[] = [
      {
        txid: "update-tx",
        action: "update",
        name: null,
        nameHash: "deadbeef",
        valueDoos: 0,
        nameValueDoos: null, // backend could not resolve the output value
        direction: "internal",
        height: 150,
        time: 1500,
        confirmed: true,
        counterparty: null,
      },
    ];
    const drafts: TxDraftSummary[] = [
      {
        id: "draft-update",
        walletProfileId: "profile1",
        action: "update",
        status: "confirmed",
        summary: {
          action: "update",
          sendTotalDoos: 222_000_000,
          feeDoos: 10_000,
          changeDoos: 0,
          inputTotalDoos: 232_000_000,
          numInputs: 1,
          recipientAddress: null,
          txid: "update-tx",
          warnings: [],
          name: "myname",
        },
        errorMessage: null,
        txid: "update-tx",
        confirmationHeight: 150,
        purchaseLostReason: null,
        createdAt: "2026-07-24 15:00:00",
      },
    ];
    const merged = mergeActivity(rows, drafts);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.nameValueDoos).toBe(222_000_000);
    // Regression: only OPEN, BID and FINALIZE carry a raw name in their
    // covenant, so the chain row for every other name action arrives nameless.
    // The draft this wallet built knows it — without the fallback the Activity
    // row showed its action and then "—" where the name belongs.
    expect(merged[0]!.name).toBe("myname");
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
        nameValueDoos: null,
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
        nameValueDoos: null,
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
        purchaseLostReason: null,
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
        purchaseLostReason: null,
        createdAt: "2026-07-24 12:00:00",
      },
    ];
    const merged = mergeActivity([], drafts);
    expect(merged[0]!.valueDoos).toBe(-100_000_000); // negative (outflow)
  });
});
