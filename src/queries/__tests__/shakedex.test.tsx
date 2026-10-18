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
  useMyListings,
  useBuildLock,
  useFinalizeAndSign,
  useCancelListing,
  useFinalizeCancel,
  useLowerPrice,
  useRestoreLock,
  useImportOwnListing,
  useSaveListingFile,
  usePublishListing,
} from "../shakedex";
import { useWithSigner } from "../wallet";
import type { MarketPage } from "../../types";

function setup() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const spy = vi.spyOn(qc, "invalidateQueries");
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  return { wrapper, spy, qc };
}

/** The active profile `id`, as `list_wallet_profiles` lists it. */
const profileRow = (id: string) => ({ id, label: id, network: "mainnet", active: true });

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
    invokeMock.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "list_wallet_profiles" ? [profileRow("p1")] : page),
    );
    const { wrapper } = setup();
    const { result } = renderHook(() => useMarketPage(3), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual(page));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_list_market", {
      page: 3,
      walletProfileId: "p1",
    });
  });

  it("useMarketPage never shows the previous profile's page while the next profile's loads (R7)", async () => {
    let active = "pA";
    const pageOf = (id: string): MarketPage => ({ ...page, rows: [{ name: id } as never] });
    let releaseB: (p: MarketPage) => void = () => {};
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profileRow(active)]);
      if (active === "pA") return Promise.resolve(pageOf("pA"));
      return new Promise<MarketPage>((r) => (releaseB = r));
    });
    const { wrapper, qc } = setup();
    const { result } = renderHook(() => useMarketPage(1), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual(pageOf("pA")));
    active = "pB";
    await act(() => qc.invalidateQueries({ queryKey: ["wallet", "profiles"] }));
    await waitFor(() => expect(result.current.data).toBeUndefined());
    act(() => releaseB(pageOf("pB")));
    await waitFor(() => expect(result.current.data).toEqual(pageOf("pB")));
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

describe("shakedex selling hooks", () => {
  it("useMyListings asks shakedex_list_listings for the active profile", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      Promise.resolve(cmd === "list_wallet_profiles" ? [profileRow("p1")] : []),
    );
    const { wrapper } = setup();
    const { result } = renderHook(() => useMyListings(), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual([]));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_list_listings", { walletProfileId: "p1" });
  });

  it("useMyListings never shows the previous profile's rows, even when its read lands late (R7)", async () => {
    let active = "pA";
    const rowsOf = (id: string) => [{ listing: { id } }];
    const pending: Record<string, (v: unknown) => void> = {};
    invokeMock.mockImplementation((cmd: string, args?: { walletProfileId?: string }) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profileRow(active)]);
      const id = args?.walletProfileId ?? active;
      return new Promise((r) => (pending[id] = r));
    });
    const { wrapper, qc } = setup();
    const { result } = renderHook(() => useMyListings(), { wrapper });
    await waitFor(() => expect(pending.pA).toBeDefined());
    // The switch happens while profile A's read is still in flight.
    active = "pB";
    await act(() => qc.invalidateQueries({ queryKey: ["wallet", "profiles"] }));
    await waitFor(() => expect(pending.pB).toBeDefined());
    act(() => pending.pB!(rowsOf("pB")));
    await waitFor(() => expect(result.current.data).toEqual(rowsOf("pB")));
    act(() => pending.pA!(rowsOf("pA")));
    await new Promise((r) => setTimeout(r, 20));
    expect(result.current.data).toEqual(rowsOf("pB"));
  });

  it("useMyListings is idle when disabled", () => {
    const { wrapper } = setup();
    renderHook(() => useMyListings(false), { wrapper });
    expect(invokeMock).not.toHaveBeenCalledWith("shakedex_list_listings", expect.anything());
  });

  it("useBuildLock asks shakedex_build_lock_draft with the mode and the publish box", async () => {
    invokeMock.mockResolvedValue({ id: "d1" });
    const { wrapper, spy } = setup();
    const { result } = renderHook(() => useBuildLock(), { wrapper });
    await act(() =>
      result.current.mutateAsync({
        name: "dexsale",
        mode: "buyNow",
        publish: false,
        feeRate: null,
      }),
    );
    expect(invokeMock).toHaveBeenCalledWith("shakedex_build_lock_draft", {
      name: "dexsale",
      mode: "buyNow",
      publish: false,
      feeRate: null,
    });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["shakedex"] });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["wallet"] });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["read"] });
  });

  it("useFinalizeAndSign sends the prices, the schedule and the fee rate", async () => {
    invokeMock.mockResolvedValue({ id: "l1" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useFinalizeAndSign(), { wrapper });
    const schedule = { startPrice: "5", floorPrice: "3", steps: 3, intervalSecs: 900 };
    await act(() =>
      result.current.mutateAsync({ listingId: "l1", prices: [], schedule, feeRate: 2000 }),
    );
    expect(invokeMock).toHaveBeenCalledWith("shakedex_finalize_and_sign", {
      listingId: "l1",
      prices: [],
      schedule,
      feeRate: 2000,
    });
  });

  it("useCancelListing sends the listing and the fee rate", async () => {
    invokeMock.mockResolvedValue({ id: "c1" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useCancelListing(), { wrapper });
    await act(() => result.current.mutateAsync({ listingId: "l1", feeRate: null }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_cancel_listing", {
      listingId: "l1",
      feeRate: null,
    });
  });

  it("useFinalizeCancel sends the listing and the fee rate", async () => {
    invokeMock.mockResolvedValue({ id: "f1" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useFinalizeCancel(), { wrapper });
    await act(() => result.current.mutateAsync({ listingId: "l1", feeRate: 1000 }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_finalize_cancel", {
      listingId: "l1",
      feeRate: 1000,
    });
  });

  it("useLowerPrice sends the price as typed", async () => {
    invokeMock.mockResolvedValue({ id: "l1" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useLowerPrice(), { wrapper });
    await act(() => result.current.mutateAsync({ listingId: "l1", price: "4.5" }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_lower_price", {
      listingId: "l1",
      price: "4.5",
    });
  });

  it("useRestoreLock sends the name", async () => {
    invokeMock.mockResolvedValue({ id: "l1" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useRestoreLock(), { wrapper });
    await act(() => result.current.mutateAsync("dexsale"));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_restore_lock", { name: "dexsale" });
  });

  it("useImportOwnListing sends pasted text or a file's path to its own command", async () => {
    invokeMock.mockResolvedValue({ id: "l1" });
    const { wrapper } = setup();
    const { result } = renderHook(() => useImportOwnListing(), { wrapper });
    await act(() => result.current.mutateAsync({ kind: "text", text: "{}" }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_import_own_listing_file", { text: "{}" });
    await act(() => result.current.mutateAsync({ kind: "file", path: "/tmp/l.json" }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_import_own_listing_from_file", {
      path: "/tmp/l.json",
    });
  });

  it("useSaveListingFile sends the listing and the chosen path", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { wrapper } = setup();
    const { result } = renderHook(() => useSaveListingFile(), { wrapper });
    await act(() => result.current.mutateAsync({ listingId: "l1", path: "/tmp/l.json" }));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_save_listing_file", {
      listingId: "l1",
      path: "/tmp/l.json",
    });
  });

  it("usePublishListing sends the listing and refreshes the listings", async () => {
    invokeMock.mockResolvedValue({ id: "l1" });
    const { wrapper, spy } = setup();
    const { result } = renderHook(() => usePublishListing(), { wrapper });
    await act(() => result.current.mutateAsync("l1"));
    expect(invokeMock).toHaveBeenCalledWith("shakedex_publish_listing", { listingId: "l1" });
    expect(spy).toHaveBeenCalledWith({ queryKey: ["shakedex"] });
  });
});

describe("useWithSigner", () => {
  it("unlocks this profile's signer once on a locked wallet and runs the command again", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { wrapper } = setup();
    const { result } = renderHook(() => useWithSigner(), { wrapper });
    const fn = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce("Wallet locked")
      .mockResolvedValueOnce("done");
    let out: string | undefined;
    await act(async () => {
      out = await result.current.run("p1", fn);
    });
    expect(out).toBe("done");
    expect(fn).toHaveBeenCalledTimes(2);
    expect(invokeMock).toHaveBeenCalledWith("unlock_local_signer", { walletProfileId: "p1" });
  });

  it("passes any other error on without unlocking", async () => {
    const { wrapper } = setup();
    const { result } = renderHook(() => useWithSigner(), { wrapper });
    const fn = vi.fn<() => Promise<string>>().mockRejectedValue("Invalid input: no");
    await expect(result.current.run("p1", fn)).rejects.toBe("Invalid input: no");
    expect(fn).toHaveBeenCalledTimes(1);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
