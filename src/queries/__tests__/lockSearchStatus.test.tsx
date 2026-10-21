import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useFindLocks, useLockSearchStatus, useRestoreLock } from "../shakedex";
import { useSyncStatus } from "../sync";
import { makeProfile } from "../../test/fixtures/wallet";

const sync = { running: false };
let report = { ran: true, adopted: [] as string[], done: false, complete: false };

const statusReads = () =>
  invokeMock.mock.calls.filter((c) => c[0] === "shakedex_lock_search_status").length;

function setup() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  const hook = renderHook(
    () => ({
      status: useLockSearchStatus(true),
      sync: useSyncStatus(),
      find: useFindLocks(),
      restore: useRestoreLock(),
    }),
    { wrapper },
  );
  return { qc, hook };
}

beforeEach(() => {
  sync.running = false;
  report = { ran: true, adopted: [], done: false, complete: false };
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([makeProfile()]);
      case "get_sync_status":
        return Promise.resolve({ running: sync.running, errors: [] });
      case "shakedex_lock_search_status":
        return Promise.resolve({ done: false, addressIndex: true, candidates: 1 });
      case "shakedex_find_locks":
        return Promise.resolve(report);
      case "shakedex_restore_lock":
        return Promise.resolve({ id: "l1", name: "dexsale" });
      default:
        return Promise.resolve(null);
    }
  });
});

// The status reads every derived address's history on the node: it is read
// again only when something it reports can have changed, never on every
// refresh of the Shakedex queries.
describe("useLockSearchStatus", () => {
  it("is not read again when a selling mutation refreshes the Shakedex queries", async () => {
    const { qc, hook } = setup();
    await waitFor(() => expect(hook.result.current.status.isSuccess).toBe(true));
    expect(statusReads()).toBe(1);

    await act(async () => {
      await hook.result.current.restore.mutateAsync("dexsale");
    });
    await act(async () => {
      await qc.invalidateQueries({ queryKey: ["shakedex"] });
    });

    expect(statusReads()).toBe(1);
  });

  it("is read again after a lock search ran", async () => {
    const { hook } = setup();
    await waitFor(() => expect(statusReads()).toBe(1));

    await act(async () => {
      await hook.result.current.find.mutateAsync(true);
    });

    await waitFor(() => expect(statusReads()).toBe(2));
  });

  it("is read again at the end of a sync", async () => {
    const { qc, hook } = setup();
    await waitFor(() => expect(hook.result.current.status.isSuccess).toBe(true));
    expect(statusReads()).toBe(1);

    sync.running = true;
    await act(async () => {
      await qc.invalidateQueries({ queryKey: ["sync", "status"] });
    });
    await waitFor(() => expect(hook.result.current.sync.data?.running).toBe(true));
    sync.running = false;
    await act(async () => {
      await qc.invalidateQueries({ queryKey: ["sync", "status"] });
    });

    await waitFor(() => expect(statusReads()).toBe(2));
  });
});
