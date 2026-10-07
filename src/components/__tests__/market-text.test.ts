import { describe, it, expect } from "vitest";
import {
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
