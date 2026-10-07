import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useMarketPage, useImportListing } from "../shakedex";
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
});
