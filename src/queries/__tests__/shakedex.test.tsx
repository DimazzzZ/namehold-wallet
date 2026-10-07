import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import {
  useMarketPage,
  useImportListing,
  usePurchasePreview,
  useBuildPurchase,
  useBuildPurchaseFinalize,
} from "../shakedex";
import type { MarketPage } from "../../types";

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const spy = vi.spyOn(qc, "invalidateQueries");
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  return { wrapper, spy };
}

const page: MarketPage = {
  rows: [],
  hidden: {
    soldOrCancelled: 0,
    failedVerification: 0,
    expiresBeforeFinalize: 0,
    notYetValid: 0,
    couldNotCheck: 0,
  },
  hiddenRows: [],
  verified: true,
  networkHasMarket: true,
  page: 1,
  pageCount: 1,
};

beforeEach(() => {
  invokeMock.mockReset();
});

describe("shakedex queries", () => {
  it("useMarketPage asks shakedex_list_market for its page", async () => {
    invokeMock.mockResolvedValue(page);
    const { wrapper } = setup();
    const { result } = renderHook(() => useMarketPage(3), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual(page));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_list_market", { page: 3 });
  });

  it("useImportListing sends the source", async () => {
    invokeMock.mockResolvedValue({ name: "x" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useImportListing(), { wrapper });
    await act(() => result.current.mutateAsync({ kind: "text", json: "{}" }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_import_listing", {
      source: { kind: "text", json: "{}" },
    });
  });

  it("usePurchasePreview passes camelCase args", async () => {
    invokeMock.mockResolvedValue({ name: "x" });
    const { wrapper } = setup();
    const { result } = renderHook(() => usePurchasePreview("{}", true, false, 5000), { wrapper });
    await waitFor(() => expect(result.current.data).toBeTruthy());
    expect(invokeMock).toHaveBeenCalledWith("shakedex_preview_purchase", {
      listingJson: "{}",
      payMarketFee: true,
      fromMarket: false,
      feeRate: 5000,
    });
  });

  it("usePurchasePreview is idle without a listing", () => {
    const { wrapper } = setup();
    renderHook(() => usePurchasePreview(null, false, false, null), { wrapper });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("useBuildPurchase sends the accepted fee and invalidates wallet", async () => {
    invokeMock.mockResolvedValue({ id: "d1" });
    const { wrapper, spy } = setup();
    const { result } = renderHook(() => useBuildPurchase(), { wrapper });
    await act(() =>
      result.current.mutateAsync({
        listingJson: "{}",
        acceptedMarketFeeDoos: 1234,
        fromMarket: true,
        feeRate: null,
      }),
    );
    expect(invokeMock).toHaveBeenCalledWith("shakedex_build_purchase_draft", {
      listingJson: "{}",
      acceptedMarketFeeDoos: 1234,
      fromMarket: true,
      feeRate: null,
    });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["wallet"] });
  });

  it("useBuildPurchaseFinalize invalidates wallet and read", async () => {
    invokeMock.mockResolvedValue({ id: "d2" });
    const { wrapper, spy } = setup();
    const { result } = renderHook(() => useBuildPurchaseFinalize(), { wrapper });
    await act(() => result.current.mutateAsync({ purchaseId: "p1", feeRate: null }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_build_purchase_finalize_draft", {
      purchaseId: "p1",
      feeRate: null,
    });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["wallet"] });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["read"] });
  });
});
