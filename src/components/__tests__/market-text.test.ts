import { describe, it, expect } from "vitest";
import {
  listingStatus,
  nodeReason,
  MAINNET_EXPERIMENTAL,
  NEEDS_SENDING_NODE,
  RECOVERY_PHRASE_ONLY,
  listedUntilText,
  purchaseRefusal,
} from "../market/marketText";

describe("listedUntilText", () => {
  it("dates the seller's expiresAt in UTC, and says the date does not end the listing", () => {
    expect(listedUntilText(1815232480)).toBe(
      "Listed until July 10, 2027 · stays buyable until sold or cancelled",
    );
  });

  it("gives no text for an expiresAt no date can hold, rather than throwing", () => {
    // expiresAt comes from a foreign listing file as any u64.
    expect(listedUntilText(Number.MAX_SAFE_INTEGER)).toBeNull();
    expect(listedUntilText(18446744073709552000)).toBeNull();
  });
});

describe("purchaseRefusal", () => {
  const send = { signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null };
  const noSend = { ...send, broadcasterAvailable: false, canWrite: false };

  it("refuses a mainnet purchase until the experimental flag is on, as the backend does", () => {
    expect(purchaseRefusal("mnemonic_hot", send, "mainnet", "false")).toBe(MAINNET_EXPERIMENTAL);
    expect(purchaseRefusal("mnemonic_hot", send, "mainnet", undefined)).toBe(MAINNET_EXPERIMENTAL);
    expect(purchaseRefusal("mnemonic_hot", send, "mainnet", "true")).toBeNull();
    expect(purchaseRefusal("mnemonic_hot", send, "regtest", "false")).toBeNull();
  });

  it("gives the profile and node refusals first, in the backend's order", () => {
    expect(purchaseRefusal("ledger_hardware", send, "mainnet", "false")).toBe(RECOVERY_PHRASE_ONLY);
    expect(purchaseRefusal("mnemonic_hot", noSend, "mainnet", "false")).toBe(NEEDS_SENDING_NODE);
  });
});

describe("nodeReason", () => {
  it("drops the layer's name from a node failure", () => {
    expect(
      nodeReason(
        "Node RPC error: the node refused the API key (HTTP 401): set the node's API key in Settings → Connections",
      ),
    ).toBe(
      "the node refused the API key (HTTP 401): set the node's API key in Settings → Connections",
    );
  });

  it("says a node that did not answer is not answering", () => {
    expect(
      nodeReason("HTTP error: error sending request for url (http://127.0.0.1:14037/)"),
    ).toMatch(/your node did not answer/);
    expect(nodeReason("HTTP error: operation timed out")).toMatch(/took too long/);
  });
});

describe("listingStatus", () => {
  const base = {
    listingJson: "{}",
    name: "n",
    kind: "buyNow",
    currentPrice: 1,
    nextPrice: null,
    nextValidInSecs: null,
    floorPrice: 1,
    steps: [],
    expiresAt: null,
  } as const;

  it("reads a buyable listing as ready, and warns when it expires soon", () => {
    const buyable = { verdict: "buyable", warnExpiry: false } as never;
    expect(listingStatus({ ...base, verdict: buyable } as never)).toEqual({
      tone: "ok",
      text: "Ready to buy",
    });
    const soon = { verdict: "buyable", warnExpiry: true } as never;
    expect(listingStatus({ ...base, verdict: soon } as never).tone).toBe("warn");
  });

  it("gives a listing that could not be checked the node's reason in plain words", () => {
    const v = {
      verdict: "hidden",
      kind: "couldNotCheck",
      reason: "Node RPC error: error sending request for url (http://127.0.0.1:14037/)",
    } as never;
    const s = listingStatus({ ...base, verdict: v } as never);
    expect(s.tone).toBe("warn");
    expect(s.text).toMatch(/^Could not be checked: your node did not answer/);
  });

  it("marks a listing that fails verification as bad", () => {
    const v = { verdict: "hidden", kind: "failedVerification", reason: "x" } as never;
    expect(listingStatus({ ...base, verdict: v } as never).tone).toBe("bad");
  });
});
