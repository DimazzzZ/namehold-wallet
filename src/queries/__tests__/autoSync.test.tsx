import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { useAutoSync, AUTO_SYNC_INTERVAL_MS } from "../autoSync";

// A mutable node/sync state the mocked backend reads, so a test can flip the
// node between "explorer" (not live) and "local" (live), and mark a sync run
// in flight, between polls.
function makeBackend() {
  const state = { readSource: "explorer" as "explorer" | "local", running: false };
  const impl = (cmd: string) => {
    switch (cmd) {
      case "node_status":
        return Promise.resolve({ connected: state.readSource === "local", read_source: state.readSource });
      case "get_sync_status":
        return Promise.resolve({ running: state.running });
      case "start_full_sync":
        return Promise.resolve({ started: !state.running, alreadyRunning: state.running });
      default:
        return Promise.resolve(null);
    }
  };
  return { state, impl };
}

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const Wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  );
  return { Wrapper };
}

const startCalls = () => invokeMock.mock.calls.filter((c) => c[0] === "start_full_sync").length;

beforeEach(() => {
  invokeMock.mockReset();
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

describe("useAutoSync", () => {
  it("does not sync while the node is not live", async () => {
    const backend = makeBackend(); // readSource: "explorer"
    invokeMock.mockImplementation(backend.impl);
    renderHook(() => useAutoSync(), { wrapper: wrapper().Wrapper });

    // Let node_status settle, then advance well past the interval.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTO_SYNC_INTERVAL_MS * 2);
    });
    expect(startCalls()).toBe(0);
  });

  it("kicks exactly one sync on the explorer -> local edge", async () => {
    const backend = makeBackend();
    invokeMock.mockImplementation(backend.impl);
    renderHook(() => useAutoSync(), { wrapper: wrapper().Wrapper });

    // Settle on not-live: no sync yet.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    expect(startCalls()).toBe(0);

    // Node becomes live → the next node_status poll (every 3s) flips the edge.
    backend.state.readSource = "local";
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_100);
    });
    expect(startCalls()).toBe(1);
  });

  it("re-kicks on the interval while live", async () => {
    const backend = makeBackend();
    backend.state.readSource = "local"; // live from the start
    invokeMock.mockImplementation(backend.impl);
    renderHook(() => useAutoSync(), { wrapper: wrapper().Wrapper });

    // Mount edge kicks once.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    expect(startCalls()).toBe(1);

    // Each subsequent interval tick kicks another sync.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTO_SYNC_INTERVAL_MS + 100);
    });
    expect(startCalls()).toBe(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTO_SYNC_INTERVAL_MS);
    });
    expect(startCalls()).toBe(3);
  });

  it("skips the interval kick while a sync is already running (guard)", async () => {
    const backend = makeBackend();
    backend.state.readSource = "local";
    backend.state.running = true; // a sync is already in flight
    invokeMock.mockImplementation(backend.impl);
    renderHook(() => useAutoSync(), { wrapper: wrapper().Wrapper });

    // Let node_status + get_sync_status settle (both observed as live+running)
    // BEFORE any edge/interval fires. useSyncStatus polls every 1.5s while
    // running, so the guard reads a fresh running=true.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_600);
    });
    // The mount edge and the interval must both be suppressed by the guard.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(AUTO_SYNC_INTERVAL_MS + 100);
    });
    expect(startCalls()).toBe(0);
  });
});
