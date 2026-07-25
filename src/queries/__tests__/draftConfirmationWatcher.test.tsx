import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Mock the invoke bridge BEFORE importing the hook (so its import chain sees
// the mock — the watcher pulls in useTxDrafts which pulls in `../lib/invoke`).
const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useDraftConfirmationWatcher } from "../wallet";

function wrapper() {
  // A single QueryClient per test — the watcher observes ["wallet","drafts"]
  // via useTxDrafts, and we seed the cache directly to simulate the
  // broadcasted→confirmed transition without waiting on the 15s poll.
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false, gcTime: Infinity } },
  });
  return {
    qc,
    Wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    ),
  };
}

const baseBroadcasted = {
  id: "d_upd",
  walletProfileId: "p1",
  action: "update",
  status: "broadcasted" as const,
  summary: { action: "update", sendTotalDoos: 0, feeDoos: 0, changeDoos: 0, inputTotalDoos: 0, numInputs: 0, recipientAddress: null, txid: "tx1", warnings: [], name: "myname" },
  errorMessage: null,
  txid: "tx1",
  confirmationHeight: null,
  createdAt: "2025-01-01",
};
const baseConfirmed = { ...baseBroadcasted, status: "confirmed" as const, confirmationHeight: 100 };

function nameRecordsInvalidations(qc: QueryClient): number {
  // Cast through unknown: `invalidateQueries` is a generic method whose mock
  // signature vitest can't cleanly express. We only care about the recorded
  // call args at runtime.
  const spy = qc.invalidateQueries as unknown as { mock?: { calls: unknown[][] } };
  const calls = spy.mock?.calls ?? [];
  return calls.filter((c) => {
    const key = (c[0] as { queryKey?: unknown[] } | undefined)?.queryKey;
    return Array.isArray(key) && key[0] === "read" && key[1] === "nameRecords";
  }).length;
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("useDraftConfirmationWatcher", () => {
  it("invalidates nameRecords on the broadcasted→confirmed edge (once, then not again)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "refresh_tx_confirmations":
          return Promise.resolve({});
        case "list_tx_drafts":
          return Promise.resolve([baseBroadcasted]);
        default:
          return Promise.resolve(null);
      }
    });
    const { qc, Wrapper } = wrapper();
    vi.spyOn(qc, "invalidateQueries");
    renderHook(() => useDraftConfirmationWatcher(), { wrapper: Wrapper });

    // First poll: the draft is broadcasted → no nameRecords invalidation yet.
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "list_tx_drafts")).toBe(true),
    );
    expect(nameRecordsInvalidations(qc)).toBe(0);

    // Next poll: same draft flips to confirmed → invalidate exactly once.
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "refresh_tx_confirmations":
          return Promise.resolve({});
        case "list_tx_drafts":
          return Promise.resolve([baseConfirmed]);
        default:
          return Promise.resolve(null);
      }
    });
    await act(async () => {
      await qc.invalidateQueries({ queryKey: ["wallet", "drafts"] });
    });
    await waitFor(() => expect(nameRecordsInvalidations(qc)).toBe(1));

    // A further poll with the draft STILL confirmed must NOT re-fire (edge,
    // not level).
    await act(async () => {
      await qc.invalidateQueries({ queryKey: ["wallet", "drafts"] });
    });
    await new Promise((r) => setTimeout(r, 50));
    expect(nameRecordsInvalidations(qc)).toBe(1);
  });

  it("does NOT invalidate when the first poll already sees a confirmed draft (app-start regression)", async () => {
    // Simulates an app reload where prevStatus is empty (was === undefined):
    // a historical confirmed UPDATE draft must not trigger any invalidation.
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "refresh_tx_confirmations":
          return Promise.resolve({});
        case "list_tx_drafts":
          return Promise.resolve([baseConfirmed]);
        default:
          return Promise.resolve(null);
      }
    });
    const { qc, Wrapper } = wrapper();
    vi.spyOn(qc, "invalidateQueries");
    renderHook(() => useDraftConfirmationWatcher(), { wrapper: Wrapper });

    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "list_tx_drafts")).toBe(true),
    );
    await new Promise((r) => setTimeout(r, 50));
    expect(nameRecordsInvalidations(qc)).toBe(0);
  });

  it("ignores non-record-writing action transitions", async () => {
    const sendBroadcasted = { ...baseBroadcasted, id: "d_send", action: "send_hns" };
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "refresh_tx_confirmations":
          return Promise.resolve({});
        case "list_tx_drafts":
          return Promise.resolve([sendBroadcasted]);
        default:
          return Promise.resolve(null);
      }
    });
    const { qc, Wrapper } = wrapper();
    vi.spyOn(qc, "invalidateQueries");
    renderHook(() => useDraftConfirmationWatcher(), { wrapper: Wrapper });
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "list_tx_drafts")).toBe(true),
    );

    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "refresh_tx_confirmations":
          return Promise.resolve({});
        case "list_tx_drafts":
          return Promise.resolve([{ ...sendBroadcasted, status: "confirmed", confirmationHeight: 100 }]);
        default:
          return Promise.resolve(null);
      }
    });
    await act(async () => {
      await qc.invalidateQueries({ queryKey: ["wallet", "drafts"] });
    });
    await new Promise((r) => setTimeout(r, 50));
    // A plain send confirming must not invalidate the nameRecords cache.
    expect(nameRecordsInvalidations(qc)).toBe(0);
  });
});
