import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";
import type { ReadContext, Settings as AppSettings } from "../../types";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { useSettingsStore } from "../../stores/settings";
import { WalletView } from "../WalletView";

function makeSettings(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    hsd_wallet_api_url: "http://127.0.0.1:12039",
    hsd_node_api_url: "http://127.0.0.1:12037",
    hsd_api_key: "",
    hsd_wallet_id: "primary",
    hsd_network: "mainnet",
    hsd_prefix: "",
    write_mode: "true",
    connection_mode: "auto_fallback",
    external_read_provider: "hnsfans",
    external_read_api_url: "https://hnsfans.com",
    external_read_watch_addresses: "[]",
    external_read_watch_names: "[]",
    remote_hsd_label: "",
    trusted_remote_hsd: "false",
    future_signer_mode: "none",
    advanced_mode: "false",
    onboarding_complete: "false",
    ...overrides,
  };
}

function seedSettings(overrides: Partial<AppSettings> = {}) {
  useSettingsStore.setState({
    settings: makeSettings(overrides),
    loaded: true,
    passphrase: "",
  });
}

/**
 * Read-only external provider context: the active provider is HNSFans, writes
 * are blocked, and the local node is unavailable so the wallet is not
 * manageable.
 */
function externalReadOnlyContext(
  overrides: Partial<ReadContext> = {},
): ReadContext {
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
    ...overrides,
  };
}

/**
 * Auto-fallback context where the local node went down and the app degraded to
 * the external read-only explorer. Writes are blocked while fallback is active.
 */
function fallbackActiveContext(
  overrides: Partial<ReadContext> = {},
): ReadContext {
  return {
    connectionMode: "auto_fallback",
    activeReadProvider: {
      kind: "external_hnsfans",
      label: "HNSFans",
      healthy: true,
      writeCapable: false,
      manageable: false,
      reason: "Local hsd unavailable; using external read-only fallback.",
    },
    fallbackActive: true,
    localNodeHealthy: false,
    walletAvailable: false,
    writeAllowed: false,
    writeReason: "Local hsd unavailable; using external read-only fallback.",
    ...overrides,
  };
}

/** Routes invoke calls by command name to the supplied fixtures. */
function routeInvoke(context: ReadContext) {
  invokeMock.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "get_read_context":
        return Promise.resolve(context);
      case "read_balance":
        return Promise.resolve({
          confirmed: 0,
          unconfirmed: 0,
          locked_confirmed: 0,
          locked_unconfirmed: 0,
        });
      case "read_names":
        return Promise.resolve([]);
      case "read_transactions":
        return Promise.resolve([]);
      case "check_connection":
        return Promise.resolve({ connected: false });
      case "get_address":
        return Promise.resolve(null);
      case "list_wallets":
        return Promise.resolve([]);
      default:
        return Promise.resolve(null);
    }
  });
}

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/wallet"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  useSettingsStore.setState({ settings: null, loaded: false, passphrase: "" });
});

describe("WalletView provider fallback / read-only behavior", () => {
  it("shows the active external provider as the data source", async () => {
    seedSettings({ connection_mode: "external_read_only" });
    routeInvoke(externalReadOnlyContext());
    render(<WalletView />, { wrapper: wrapper() });

    expect(await screen.findByText("HNSFans")).toBeInTheDocument();
    expect(screen.getByText(/Data source:/)).toBeInTheDocument();
  });

  it("surfaces a read-only badge with the blocked-write reason", async () => {
    seedSettings({ connection_mode: "external_read_only" });
    routeInvoke(externalReadOnlyContext());
    render(<WalletView />, { wrapper: wrapper() });

    expect(
      await screen.findByText(
        /Read-only — External read-only provider does not support writes\./,
      ),
    ).toBeInTheDocument();
  });

  it("hides the Send HNS action when writes are not allowed", async () => {
    seedSettings({ connection_mode: "external_read_only" });
    routeInvoke(externalReadOnlyContext());
    render(<WalletView />, { wrapper: wrapper() });

    // Wait for the provider context to resolve.
    await screen.findByText("HNSFans");
    expect(
      screen.queryByRole("button", { name: /Send HNS/ }),
    ).not.toBeInTheDocument();
  });

  it("hides wallet-management actions when the provider is not manageable", async () => {
    seedSettings({ connection_mode: "external_read_only" });
    routeInvoke(externalReadOnlyContext());
    render(<WalletView />, { wrapper: wrapper() });

    await screen.findByText("HNSFans");
    expect(
      screen.queryByRole("button", { name: /Create Wallet/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Switch Wallet/ }),
    ).not.toBeInTheDocument();
    // Refresh remains available even in read-only mode.
    expect(
      screen.getByRole("button", { name: /Refresh/ }),
    ).toBeInTheDocument();
  });

  it("blocks writes and explains the fallback while auto-fallback is degraded", async () => {
    seedSettings({ connection_mode: "auto_fallback" });
    routeInvoke(fallbackActiveContext());
    render(<WalletView />, { wrapper: wrapper() });

    expect(
      await screen.findByText(
        /Local hsd unavailable; using external read-only fallback\./,
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Send HNS/ }),
    ).not.toBeInTheDocument();
  });

  it("allows Send HNS when a write-capable local provider is active and write mode is on", async () => {
    seedSettings({ connection_mode: "local_managed_hsd", write_mode: "true" });
    routeInvoke({
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
    });
    render(<WalletView />, { wrapper: wrapper() });

    expect(
      await screen.findByRole("button", { name: /Send HNS/ }),
    ).toBeInTheDocument();
  });

  it("blocks writes when the provider is write-capable but write mode is disabled", async () => {
    seedSettings({ connection_mode: "local_managed_hsd", write_mode: "false" });
    routeInvoke({
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
    });
    render(<WalletView />, { wrapper: wrapper() });

    // Manage actions are still visible (local provider is manageable)...
    expect(
      await screen.findByRole("button", { name: /Create Wallet/ }),
    ).toBeInTheDocument();
    // ...but Send HNS is gated by the disabled write_mode preference.
    expect(
      screen.queryByRole("button", { name: /Send HNS/ }),
    ).not.toBeInTheDocument();
  });
});
