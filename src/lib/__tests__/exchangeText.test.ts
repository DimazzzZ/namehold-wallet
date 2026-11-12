import { describe, it, expect } from "vitest";
import {
  EXCHANGE_CHECKING,
  EXCHANGE_NOT_ENABLED,
  EXCHANGE_UNAVAILABLE,
  OWN_OFFER,
  exchangeActionRefusal,
  exchangeRefusal,
  makerRole,
  takeRefusal,
  takerRole,
} from "../exchangeText";
import { makeOffer, makeOverview } from "../../test/fixtures/exchange";

const REFUSAL = "The exchange needs a recovery-phrase wallet; this one is watch-only.";

describe("exchangeRefusal", () => {
  it("refuses while the overview has not answered", () => {
    expect(exchangeRefusal(undefined)).toBe(EXCHANGE_CHECKING);
    expect(exchangeRefusal(null)).toBe(EXCHANGE_CHECKING);
  });

  it("shows the backend's refusal verbatim when unavailable", () => {
    expect(exchangeRefusal(makeOverview({ available: false, refusal: REFUSAL }))).toBe(REFUSAL);
  });

  it("never leaves an unavailable exchange unexplained", () => {
    expect(exchangeRefusal(makeOverview({ available: false, refusal: null }))).toBe(
      EXCHANGE_UNAVAILABLE,
    );
  });

  it("allows an available exchange, enabled or not", () => {
    expect(exchangeRefusal(makeOverview())).toBeNull();
    expect(exchangeRefusal(makeOverview({ enabled: false }))).toBeNull();
  });
});

describe("exchangeActionRefusal", () => {
  it("puts the backend's refusal first, then not enabled", () => {
    expect(
      exchangeActionRefusal(makeOverview({ available: false, refusal: REFUSAL, enabled: false })),
    ).toBe(REFUSAL);
    expect(exchangeActionRefusal(makeOverview({ enabled: false }))).toBe(EXCHANGE_NOT_ENABLED);
    expect(exchangeActionRefusal(makeOverview())).toBeNull();
    expect(exchangeActionRefusal(undefined)).toBe(EXCHANGE_CHECKING);
  });
});

describe("takeRefusal", () => {
  it("refuses the user's own offer, after the gate", () => {
    expect(takeRefusal(makeOverview(), makeOffer({ own: true }))).toBe(OWN_OFFER);
    expect(takeRefusal(makeOverview(), makeOffer())).toBeNull();
    expect(
      takeRefusal(makeOverview({ available: false, refusal: REFUSAL }), makeOffer({ own: true })),
    ).toBe(REFUSAL);
  });
});

describe("roles", () => {
  it("taking a sell offer makes the user the buyer, and the reverse", () => {
    expect(takerRole("sell")).toBe("buyer");
    expect(takerRole("buy")).toBe("seller");
    expect(makerRole("sell")).toBe("seller");
    expect(makerRole("buy")).toBe("buyer");
  });
});
