import type {
  MarketPage,
  MarketRow,
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
    blocksUntilFinalize: null,
    blocksUntilUnbuyable: null,
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
    publish: ok,
    importFile: ok,
    manageHere: ok,
    ...over,
  };
}

/** A complete My listings row in `state`, every action allowed, nothing to send. */
export function makeMyListing(
  state: ShakedexListingState = "listed",
  over: Partial<ShakedexMyListing> = {},
): ShakedexMyListing {
  return {
    listing: makeListing({ state }),
    actions: makeActions(),
    prices: { kind: "noSteps" },
    pendingSend: null,
    managedElsewhere: false,
    foreignStepPriceDoos: null,
    soldElsewhere: false,
    cancelledElsewhere: false,
    unbuyableWarnBlocks: 4_320,
    wallClockTiming: true,
    ...over,
  };
}

/**
 * A buyable market row as `commands/shakedex.rs::market_row` writes it: a
 * Buy Now at 250 HNS, one step valid now. `listingJson` carries the name so
 * two rows never share a key.
 */
export function makeMarketRow(over: Partial<MarketRow> = {}): MarketRow {
  const name = over.name ?? "dexreviews";
  return {
    listingJson: JSON.stringify({ version: 2, name }),
    name,
    verdict: {
      verdict: "buyable",
      currentStep: 0,
      nextStep: null,
      lockValue: 250_000_000,
      nameHeight: 1,
      expiryEnd: 100_000,
      warnExpiry: false,
      mtp: 0,
      tip: 0,
    },
    kind: "buyNow",
    currentPrice: 250_000_000,
    nextPrice: null,
    nextValidInSecs: null,
    floorPrice: 250_000_000,
    steps: [{ price: 250_000_000, lockTime: 1_700_000_000, validInSecs: 0 }],
    expiresAt: null,
    own: false,
    ...over,
  };
}

/** A reverse auction at 1,200 HNS now, then 1,100 in 6 h, down to 400 in 3 days. */
export function makeAuctionRow(over: Partial<MarketRow> = {}): MarketRow {
  const prices: [number, number][] = [
    [1_200_000_000, 0],
    [1_100_000_000, 21_600],
    [800_000_000, 86_400],
    [400_000_000, 259_200],
  ];
  return makeMarketRow({
    name: "coffee",
    kind: "reverseAuction",
    currentPrice: 1_200_000_000,
    nextPrice: 1_100_000_000,
    nextValidInSecs: 21_600,
    floorPrice: 400_000_000,
    steps: prices.map(([price, validInSecs]) => ({
      price,
      lockTime: 1_700_000_000 + validInSecs,
      validInSecs,
    })),
    verdict: {
      verdict: "buyable",
      currentStep: 0,
      nextStep: 1,
      lockValue: 1_200_000_000,
      nameHeight: 1,
      expiryEnd: 100_000,
      warnExpiry: false,
      mtp: 0,
      tip: 0,
    },
    ...over,
  });
}

/** One page of LearnHNS Market (`shakedex_list_market`), one buyable row, nothing hidden. */
export function makeMarketPage(over: Partial<MarketPage> = {}): MarketPage {
  return {
    rows: [makeMarketRow()],
    hidden: {
      soldOrCancelled: 0,
      failedVerification: 0,
      expiresBeforeFinalize: 0,
      notYetValid: 0,
      couldNotCheck: 0,
    },
    verified: true,
    networkHasMarket: true,
    hiddenRows: [],
    page: 1,
    pageCount: 1,
    ...over,
  };
}
