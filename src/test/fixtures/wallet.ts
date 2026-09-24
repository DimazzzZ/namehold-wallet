import type { SignerSessionSummary, WalletProfileSummary } from "../../types";

/**
 * A complete `WalletProfileSummary`; each test overrides only what it is
 * about. Complete on purpose (see `capabilities.ts`): adding a field to the
 * type is a type error here and nowhere else.
 */
export function makeProfile(over: Partial<WalletProfileSummary> = {}): WalletProfileSummary {
  return {
    id: "p1",
    label: "Primary",
    kind: "mnemonic_hot",
    network: "mainnet",
    accountXpub: "xpubFAKE",
    accountIndex: 0,
    receiveDepth: 1,
    changeDepth: 0,
    receiveAddress: "hs1qfake",
    lastSyncedHeight: null,
    lastSyncedAt: null,
    lastExplorerSyncAt: null,
    watchOnly: false,
    hasPassphrase: true,
    active: true,
    ...over,
  };
}

/** A complete `SignerSessionSummary`, locked unless told otherwise. */
export function makeSession(over: Partial<SignerSessionSummary> = {}): SignerSessionSummary {
  return {
    walletProfileId: "p1",
    unlocked: false,
    unlockedUntilEpochMs: 0,
    ...over,
  };
}
