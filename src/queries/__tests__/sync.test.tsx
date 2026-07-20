import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useSyncStatus, useCancelFullSync, type SyncStatus } from "../sync";

function baseStatus(overrides: Partial<SyncStatus>): SyncStatus {
  return {
    running: false,
    step: "idle",
    progressLabel: "",
    repaired: 0,
    repairCandidates: 0,
    repairRemaining: 0,
    discovered: 0,
    namesSynced: 0,
    errors: [],
    startedAt: null,
    finishedAt: null,
    discoverAddressesTotal: 0,
    discoverAddressesDone: 0,
    discoverTxsScanned: 0,
    discoverCandidates: 0,
    discoverCurrentName: "",
    waiting: false,
    cancelRequested: false,
    ...overrides,
  };
}

beforeEach(() => invokeMock.mockReset());

describe("useSyncStatus — invalidates read/wallet caches when a sync run completes", () => {
  it("invalidates ['read'] and ['wallet'] on the running:true -> running:false transition", async () => {
    let running = true;
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_sync_status") {
        return Promise.resolve(baseStatus({ running }));
      }
      return Promise.resolve(null);
    });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useSyncStatus(), { wrapper: Wrapper });

    // First render settles on running:true — no invalidation should happen yet
    // (this is not a completion, and there's no prior "running" state to
    // transition down from).
    await waitFor(() => expect(result.current.data?.running).toBe(true));
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: ["read"] });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: ["wallet"] });

    // Simulate the next poll returning running:false (the background sync
    // just finished) by flipping the mocked response and forcing a refetch,
    // the same way `refetchInterval` would.
    running = false;
    await act(async () => {
      await result.current.refetch();
    });

    await waitFor(() => expect(result.current.data?.running).toBe(false));
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["read"] });
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["wallet"] });
  });

  it("does not invalidate on first render when sync is already idle (running:false from the start)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "get_sync_status") {
        return Promise.resolve(baseStatus({ running: false }));
      }
      return Promise.resolve(null);
    });

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useSyncStatus(), { wrapper: Wrapper });

    await waitFor(() => expect(result.current.data?.running).toBe(false));
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: ["read"] });
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: ["wallet"] });
  });
});

describe("useCancelFullSync", () => {
  it("invokes cancel_full_sync and invalidates ['sync', 'status'] on success", async () => {
    invokeMock.mockResolvedValue(null);

    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const invalidateSpy = vi.spyOn(qc, "invalidateQueries");
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    );

    const { result } = renderHook(() => useCancelFullSync(), { wrapper: Wrapper });

    await act(async () => {
      await result.current.mutateAsync();
    });

    expect(invokeMock).toHaveBeenCalledWith("cancel_full_sync", undefined);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["sync", "status"] });
  });
});
