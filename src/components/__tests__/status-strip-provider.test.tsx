import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";
import type { ReadContext } from "../../types";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { StatusStrip } from "../ui/StatusStrip";

interface InvokeFixtures {
  readContext?: ReadContext | null;
  nodeRunning?: boolean;
  walletConnected?: boolean;
  hsdVersion?: string;
  namebaseConnected?: boolean;
}

/** Routes the StatusStrip's three queries to deterministic fixtures. */
function routeInvoke(fixtures: InvokeFixtures = {}) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "get_read_context":
        return Promise.resolve(fixtures.readContext ?? null);
      case "get_node_status":
        return Promise.resolve({
          running: fixtures.nodeRunning ?? false,
          wallet_connected: fixtures.walletConnected ?? false,
          hsd_version: fixtures.hsdVersion,
        });
      case "get_namebase_status":
        return Promise.resolve({
          connected: fixtures.namebaseConnected ?? false,
          has_cookie: false,
        });
      default:
        return Promise.resolve(null);
    }
  });
}

function localHealthyContext(over: Partial<ReadContext> = {}): ReadContext {
  return {
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
    writeReason: null,
    ...over,
  };
}

function externalContext(over: Partial<ReadContext> = {}): ReadContext {
  return {
    connectionMode: "external_read_only",
    activeReadProvider: {
      kind: "external_hnsfans",
      label: "HNSFans",
      healthy: true,
      writeCapable: false,
      manageable: false,
      reason: "External read-only provider does not support writes.",
    },
    fallbackActive: false,
    localNodeHealthy: false,
    walletAvailable: false,
    writeAllowed: false,
    writeReason: "External read-only provider does not support writes.",
    ...over,
  };
}

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("StatusStrip provider indicator", () => {
  it("shows a healthy local provider as Connected", async () => {
    routeInvoke({
      readContext: localHealthyContext(),
      nodeRunning: true,
      walletConnected: true,
    });
    render(<StatusStrip />, { wrapper: wrapper() });

    const providerButton = (await screen.findByText("Provider:"))
      .closest("button") as HTMLElement;
    expect(within(providerButton).getByText("Connected")).toBeInTheDocument();
  });

  it("shows an external provider as Read-only", async () => {
    routeInvoke({ readContext: externalContext() });
    render(<StatusStrip />, { wrapper: wrapper() });

    expect(await screen.findByText("Provider:")).toBeInTheDocument();
    expect(screen.getByText("Read-only")).toBeInTheDocument();
  });

  it("shows Read-only (fallback) when auto-fallback is degraded", async () => {
    routeInvoke({
      readContext: externalContext({
        connectionMode: "auto_fallback",
        fallbackActive: true,
        activeReadProvider: {
          kind: "external_hnsfans",
          label: "HNSFans",
          healthy: true,
          writeCapable: false,
          manageable: false,
          reason: "Local hsd unavailable; using external read-only fallback.",
        },
      }),
    });
    render(<StatusStrip />, { wrapper: wrapper() });

    expect(
      await screen.findByText("Read-only (fallback)"),
    ).toBeInTheDocument();
  });

  it("shows Unavailable when the active provider is unhealthy", async () => {
    routeInvoke({
      readContext: localHealthyContext({
        activeReadProvider: {
          kind: "local_hsd",
          label: "Local hsd",
          healthy: false,
          writeCapable: false,
          manageable: true,
          reason: "hsd is not responding.",
        },
        localNodeHealthy: false,
        writeAllowed: false,
        writeReason: "hsd is not responding.",
      }),
    });
    render(<StatusStrip />, { wrapper: wrapper() });

    expect(await screen.findByText("Unavailable")).toBeInTheDocument();
  });

  it("reflects node, wallet, and namebase connectivity alongside the provider", async () => {
    routeInvoke({
      readContext: localHealthyContext(),
      nodeRunning: true,
      walletConnected: true,
      namebaseConnected: true,
    });
    render(<StatusStrip />, { wrapper: wrapper() });

    // Node status resolves asynchronously; wait for the running state.
    expect(await screen.findByText("Running")).toBeInTheDocument();

    const nodeButton = screen.getByText("Node:").closest("button") as HTMLElement;
    expect(within(nodeButton).getByText("Running")).toBeInTheDocument();

    const walletButton = screen
      .getByText("Wallet:")
      .closest("button") as HTMLElement;
    expect(within(walletButton).getByText("Connected")).toBeInTheDocument();

    expect(screen.getByText("Namebase:")).toBeInTheDocument();
  });

  it("renders node/wallet/namebase items even before the read context resolves", async () => {
    routeInvoke({ readContext: null, nodeRunning: false });
    render(<StatusStrip />, { wrapper: wrapper() });

    // No provider item without a resolved read context...
    expect(await screen.findByText("Node:")).toBeInTheDocument();
    expect(screen.getByText("Stopped")).toBeInTheDocument();
    expect(screen.getByText("Wallet:")).toBeInTheDocument();
    expect(screen.getByText("Offline")).toBeInTheDocument();
    expect(screen.queryByText("Provider:")).not.toBeInTheDocument();
  });
});
