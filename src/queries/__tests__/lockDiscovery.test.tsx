import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { useLockDiscovery } from "../lockDiscovery";
import { useUiStore } from "../../stores/ui";
import { makeProfile, makeSession } from "../../test/fixtures/wallet";

const state = { running: false, live: true, unlocked: true, kind: "mnemonic_hot" as string };
let report = { ran: true, adopted: [] as string[], done: true, complete: true };

const calls = (cmd: string) => invokeMock.mock.calls.filter((c) => c[0] === cmd);
const toasts = () => useUiStore.getState().toastQueue.map((t) => t.message);

function setup() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const spy = vi.spyOn(qc, "invalidateQueries");
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  const hook = renderHook(() => useLockDiscovery(), { wrapper });
  return { qc, spy, hook };
}

/** Flip `running` and let the status poll and the hook see it. */
async function setRunning(qc: QueryClient, running: boolean) {
  state.running = running;
  await act(async () => {
    await qc.invalidateQueries({ queryKey: ["sync", "status"] });
    await new Promise((r) => setTimeout(r, 20));
  });
}

beforeEach(() => {
  Object.assign(state, { running: false, live: true, unlocked: true, kind: "mnemonic_hot" });
  report = { ran: true, adopted: [], done: true, complete: true };
  useUiStore.setState({ toastQueue: [], toastMessage: null });
  invokeMock.mockReset();
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([makeProfile({ kind: state.kind as "mnemonic_hot" })]);
      case "get_signer_session":
        return Promise.resolve(makeSession({ unlocked: state.unlocked }));
      case "node_status":
        return Promise.resolve({
          connected: state.live,
          read_source: state.live ? "local" : "explorer",
        });
      case "get_sync_status":
        return Promise.resolve({ running: state.running });
      case "shakedex_find_locks":
        return Promise.resolve(report);
      default:
        return Promise.resolve(null);
    }
  });
});

/** Run one sync to its end, with the queries it reads settled first. */
async function oneSync(qc: QueryClient) {
  await setRunning(qc, true);
  await setRunning(qc, false);
}

describe("useLockDiscovery", () => {
  it("searches once when a sync ends with the signer unlocked", async () => {
    const { qc } = setup();
    await waitFor(() => expect(calls("get_signer_session")).toHaveLength(1));
    await waitFor(() => expect(calls("node_status")).toHaveLength(1));
    await act(async () => {});
    await oneSync(qc);
    await waitFor(() => expect(calls("shakedex_find_locks")).toHaveLength(1));
    expect(calls("shakedex_find_locks")[0]).toEqual(["shakedex_find_locks", { force: false }]);
    await act(async () => {});
    expect(calls("shakedex_find_locks")).toHaveLength(1);
    await oneSync(qc);
    await waitFor(() => expect(calls("shakedex_find_locks")).toHaveLength(2));
  });

  it("does not search while the signer is locked or the node is not live", async () => {
    state.unlocked = false;
    const first = setup();
    await act(async () => {});
    await oneSync(first.qc);
    await act(async () => {});
    expect(calls("shakedex_find_locks")).toHaveLength(0);
    first.hook.unmount();

    state.unlocked = true;
    state.live = false;
    const second = setup();
    await act(async () => {});
    await oneSync(second.qc);
    await act(async () => {});
    expect(calls("shakedex_find_locks")).toHaveLength(0);
    second.hook.unmount();

    state.live = true;
    state.kind = "watch_only";
    const third = setup();
    await act(async () => {});
    await oneSync(third.qc);
    await act(async () => {});
    expect(calls("shakedex_find_locks")).toHaveLength(0);
  });

  it("toasts and refreshes My listings when it restored a name", async () => {
    report = { ran: true, adopted: ["dexsale", "other"], done: true, complete: true };
    const { qc, spy } = setup();
    await act(async () => {});
    await oneSync(qc);
    await waitFor(() =>
      expect(toasts()).toContain("Found 2 names locked for sale · restored in My listings"),
    );
    expect(spy).toHaveBeenCalledWith(expect.objectContaining({ queryKey: ["shakedex"] }));
  });

  it("stays silent when the search is refused", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "shakedex_find_locks") return Promise.reject("Wallet locked");
      if (cmd === "list_wallet_profiles") return Promise.resolve([makeProfile()]);
      if (cmd === "get_signer_session") return Promise.resolve(makeSession({ unlocked: true }));
      if (cmd === "node_status") return Promise.resolve({ connected: true, read_source: "local" });
      if (cmd === "get_sync_status") return Promise.resolve({ running: state.running });
      return Promise.resolve(null);
    });
    const { qc } = setup();
    await act(async () => {});
    await oneSync(qc);
    await waitFor(() => expect(calls("shakedex_find_locks")).toHaveLength(1));
    expect(toasts()).toEqual([]);
  });
});
