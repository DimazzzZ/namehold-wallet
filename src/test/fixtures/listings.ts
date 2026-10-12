import type {
  ShakedexListingActions,
  ShakedexListingState,
  ShakedexListingSummary,
  ShakedexMyListing,
} from "../../types";

/**
 * A complete `ShakedexListingSummary`; each test overrides only what it is
 * about. Complete on purpose: adding a field to the type is a type error
 * here and nowhere else.
 */
export function makeListing(over: Partial<ShakedexListingSummary> = {}): ShakedexListingSummary {
  return {
    id: "l1",
    name: "dexsale",
    mode: "buyNow",
    state: "listed",
    lockTxid: "11".repeat(32),
    lockVout: 0,
    paymentAddress: "rs1qpay",
    steps: [{ price: 5_000_000, lockTime: 1_700_000_000, signature: "00" }],
    expiresAt: 1_731_536_000,
    finalizeDraftId: null,
    cancelDraftId: null,
    cancelFinalizeDraftId: null,
    cancelBlocksRemaining: null,
    marketStatus: null,
    marketError: null,
    marketRetryAt: null,
    publish: false,
    ...over,
  };
}

const ok = { allowed: true, reason: null };

/** Every action allowed; a test refuses the ones it is about. */
export function makeActions(over: Partial<ShakedexListingActions> = {}): ShakedexListingActions {
  return {
    finalizeAndSign: ok,
    lowerPrice: ok,
    cancel: ok,
    finalizeCancel: ok,
    export: ok,
    ...over,
  };
}

/** A complete My listings row in `state`, every action allowed, nothing to send. */
export function makeMyListing(
  state: ShakedexListingState = "listed",
  over: Partial<ShakedexMyListing> = {},
): ShakedexMyListing {
  return { listing: makeListing({ state }), actions: makeActions(), pendingSend: null, ...over };
}
