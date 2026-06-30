import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// Mock the Tauri invoke layer so the provider-aware queries resolve
// deterministically without a backend.
const invokeMock = vi.fn();
vi.mock("../lib/invoke", () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

// Settings store is read by useWalletReadModel for watch addresses.
vi.mock("../stores/settings", () => ({
  useSettingsStore: (selector: (s: unknown) => unknown) =>
    selector({ settings: { external_read_watch_addresses: '["hs1qwatch"]' } }),
}));

import {
  useProviderHealth,
  useReadBalance,
  useReadContext,
  useWalletReadModel,
} from "./read";

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
  };
}

const localContext = {
  connectionMode: "local_managed_hsd",
  activeReadProvider: {
    kind: "local_hsd",
    label: "Local hsd",
    healthy: true,
    writeCapable: true,
    manageable: true,
  },
  fallbackActive: false,
  localNodeHealthy: true,
  walletAvailable: true,
  writeAllowed: true,
};

const fallbackContext = {
  connectionMode: "auto_fallback",
  activeReadProvider: {
    kind: "external_hnsfans",
    label: "HNSFans",
    healthy: true,
    writeCapable: false,
    manageable: false,
    reason: "Local node is unavailable; using read-only explorer.",
  },
  fallbackActive: true,
  localNodeHealthy: false,
  walletAvailable: false,
  writeAllowed: false,
  writeReason: "Local node is unavailable; using read-only explorer.",
};

describe("useReadContext", () => {
  beforeEach(() => invokeMock.mockReset());

  it("resolves the active read context from the backend", async () => {
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "get_read_context" ? Promise.resolve(localContext) : Promise.resolve(null),
    );
    const { result } = renderHook(() => useReadContext(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.activeReadProvider?.kind).toBe("local_hsd");
    expect(result.current.data?.writeAllowed).toBe(true);
  });
});

describe("useProviderHealth", () => {
  beforeEach(() => invokeMock.mockReset());

  it("selects the active provider out of the read context", async () => {
    invokeMock.mockResolvedValue(fallbackContext);
    const { result } = renderHook(() => useProviderHealth(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.kind).toBe("external_hnsfans");
    expect(result.current.data?.healthy).toBe(true);
    expect(result.current.data?.writeCapable).toBe(false);
  });

  it("returns null when there is no active provider", async () => {
    invokeMock.mockResolvedValue({ ...localContext, activeReadProvider: null });
    const { result } = renderHook(() => useProviderHealth(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toBeNull();
  });
});

describe("useReadBalance", () => {
  beforeEach(() => invokeMock.mockReset());

  it("normalizes a missing balance to null", async () => {
    invokeMock.mockResolvedValue(undefined);
    const { result } = renderHook(() => useReadBalance(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toBeNull();
  });
});

describe("useWalletReadModel (fallback path)", () => {
  beforeEach(() => invokeMock.mockReset());

  it("builds a read-only model when the active provider is an external fallback", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "get_read_context":
          return Promise.resolve(fallbackContext);
        case "read_balance":
          return Promise.resolve({
            confirmed: 1_000_000,
            unconfirmed: 0,
            locked_confirmed: null,
            locked_unconfirmed: null,
          });
        case "read_names":
          return Promise.resolve([]);
        case "read_transactions":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    });

    const { result } = renderHook(() => useWalletReadModel(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const model = result.current.data!;
    expect(model.context.fallbackActive).toBe(true);
    expect(model.context.activeReadProvider?.kind).toBe("external_hnsfans");
    expect(model.watchAddresses).toEqual(["hs1qwatch"]);
    expect(model.readOnlyReason).toBe(
      "Local node is unavailable; using read-only explorer.",
    );
    expect(model.balance?.confirmed).toBe(1_000_000);
  });

  it("tolerates individual read failures without rejecting the model", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_read_context") return localContext;
      // Sub-reads return null / non-array junk; the model normalizes these to
      // null balance and empty name/transaction lists rather than rejecting.
      return null;
    });

    const { result } = renderHook(() => useWalletReadModel(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const model = result.current.data!;
    expect(model.balance).toBeNull();
    expect(model.names).toEqual([]);
    expect(model.transactions).toEqual([]);
    expect(model.readOnlyReason).toBeNull();
  });
});
