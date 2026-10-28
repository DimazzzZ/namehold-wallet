import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Navigate, Route, Routes, useLocation } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn(), readText: vi.fn() }));
const counts = vi.hoisted(() => ({ names: 0, activity: 0 }));
vi.mock("../../hooks/useNavCounts", () => ({ useNavCounts: () => counts }));

import { WalletSwitcher } from "./WalletSwitcher";
import { Layout } from "../Layout";
import { useUiStore } from "../../stores/ui";
import { netTheme } from "../../lib/networkTheme";
import { ROUTE_ALIASES } from "../../lib/navigation";

const p = (id: string, label: string, over: Record<string, unknown> = {}) => ({
  id,
  label,
  kind: "mnemonic_hot",
  network: "mainnet",
  watchOnly: false,
  hasPassphrase: true,
  active: false,
  lastSyncedAt: null,
  lastExplorerSyncAt: null,
  lastSyncedHeight: null,
  ...over,
});

function route(profiles: unknown[]) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve(profiles);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: null, unlocked: false, unlockedUntilEpochMs: 0 });
      default:
        return Promise.resolve(null);
    }
  };
}

function Where() {
  const l = useLocation();
  return <div data-testid="where">{l.pathname + l.search}</div>;
}

function wrap(initial = "/") {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={[initial]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  counts.names = 0;
  counts.activity = 0;
  useUiStore.setState({ walletsDialog: null, walletMenuOpen: false, readinessOpen: false });
});

describe("netTheme", () => {
  it("reads hsd's 'main' as mainnet, and only test networks get a strip", () => {
    expect(netTheme("main").label).toBe("Mainnet");
    expect(netTheme("mainnet").strip).toBeNull();
    expect(netTheme("testnet").strip).not.toBeNull();
    expect(netTheme("regtest").strip).not.toBeNull();
    expect(netTheme("simnet").strip).not.toBeNull();
  });
  it("falls back to a neutral theme for an unknown network", () => {
    const t = netTheme("weirdnet");
    expect(t.label).toBe("weirdnet");
    expect(t.dot).toBe("bg-gray-400");
    expect(t.strip).toBeNull();
  });
});

describe("WalletSwitcher", () => {
  it("lists every wallet, ticks the active one and switches on pick", async () => {
    invokeMock.mockImplementation(
      route([p("p1", "Primary", { active: true }), p("p2", "Trading", { network: "testnet" })]),
    );
    render(<WalletSwitcher />, { wrapper: wrap() });
    const card = await screen.findByTestId("wallet-switcher");
    await waitFor(() => expect(card).toHaveTextContent("Primary"));
    expect(card).toHaveTextContent("Hot ·Mainnet");
    fireEvent.click(card);
    const items = screen.getAllByTestId("wallet-menu-item");
    expect(items.map((i) => i.textContent)).toEqual(["PrimaryHot ·Mainnet", "TradingHot ·Testnet"]);
    expect(items[0]!.querySelector("svg")).not.toBeNull(); // the ✓
    fireEvent.click(items[1]!);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("set_active_wallet_profile", {
        walletProfileId: "p2",
      }),
    );
  });

  it("Add wallet and Manage wallets open the one Wallets dialog", async () => {
    invokeMock.mockImplementation(route([p("p1", "Primary", { active: true })]));
    render(<WalletSwitcher />, { wrapper: wrap() });
    fireEvent.click(await screen.findByTestId("wallet-switcher"));
    fireEvent.click(screen.getByRole("menuitem", { name: /Add wallet/ }));
    expect(useUiStore.getState().walletsDialog).toBe("add");
    fireEvent.click(screen.getByTestId("wallet-switcher"));
    fireEvent.click(screen.getByRole("menuitem", { name: /Manage wallets/ }));
    expect(useUiStore.getState().walletsDialog).toBe("list");
  });

  it("shows the lock for a hot wallet only", async () => {
    invokeMock.mockImplementation(route([p("p1", "Primary", { active: true })]));
    const { unmount } = render(<WalletSwitcher />, { wrapper: wrap() });
    expect(await screen.findByTestId("sidebar-lock")).toHaveAccessibleName("Unlock the signer");
    unmount();
    for (const over of [{ watchOnly: true }, { kind: "ledger_hardware" }]) {
      invokeMock.mockImplementation(route([p("p1", "W", { active: true, ...over })]));
      const r = render(<WalletSwitcher />, { wrapper: wrap() });
      await waitFor(() => expect(screen.getByTestId("wallet-switcher")).toHaveTextContent("W"));
      expect(screen.queryByTestId("sidebar-lock")).toBeNull();
      r.unmount();
    }
  });

  it("says No wallet with none", async () => {
    invokeMock.mockImplementation(route([]));
    render(<WalletSwitcher />, { wrapper: wrap() });
    expect(await screen.findByTestId("wallet-switcher")).toHaveTextContent(
      "No walletCreate or import one",
    );
    fireEvent.click(screen.getByTestId("wallet-switcher"));
    expect(screen.getByText("No wallets yet.")).toBeInTheDocument();
  });
});

describe("Layout", () => {
  const renderLayout = (profiles: unknown[], initial = "/") => {
    invokeMock.mockImplementation(route(profiles));
    return render(
      <Routes>
        <Route element={<Layout />}>
          <Route path="*" element={<Where />} />
        </Route>
      </Routes>,
      { wrapper: wrap(initial) },
    );
  };

  it("has five nav items in order, a Beta badge and no Beta banner", async () => {
    renderLayout([p("p1", "Primary", { active: true })]);
    const labels = Array.from(document.querySelectorAll("nav a")).map((a) =>
      a.textContent?.replace("New", ""),
    );
    expect(labels).toEqual(["Wallet", "Names", "Market", "Activity", "Settings"]);
    expect(screen.getByTestId("beta-badge")).toBeInTheDocument();
    expect(screen.queryByTestId("beta-banner")).toBeNull();
  });

  it("shows the nav counts and hides them at 0", async () => {
    counts.names = 3;
    renderLayout([p("p1", "Primary", { active: true })]);
    expect(screen.getByTestId("nav-count-names")).toHaveTextContent("3");
    expect(screen.queryByTestId("nav-count-activity")).toBeNull();
  });

  it("keeps Names active on a name's page", () => {
    renderLayout([], "/names/coffee");
    const names = within(document.querySelector("nav")!).getByText("Names").closest("a")!;
    expect(names.className).toContain("bg-blue-50");
  });

  it("shows the QA banner in a plain browser, not in the desktop app", () => {
    const w = window as unknown as Record<string, unknown>;
    const saved = w.__TAURI_INTERNALS__;
    const { unmount } = renderLayout([]);
    expect(screen.queryByTestId("web-qa-banner")).toBeNull();
    unmount();
    delete w.__TAURI_INTERNALS__;
    try {
      renderLayout([]);
      expect(screen.getByTestId("web-qa-banner")).toBeInTheDocument();
    } finally {
      w.__TAURI_INTERNALS__ = saved;
    }
  });

  it("shows the network strip only for a test network", async () => {
    const { unmount } = renderLayout([p("p1", "Dev", { active: true, network: "regtest" })]);
    expect(await screen.findByTestId("network-strip")).toHaveTextContent(
      "RegtestLocal test chain — blocks are mined on demand",
    );
    unmount();
    renderLayout([p("p1", "Primary", { active: true })]);
    await waitFor(() => expect(screen.getByTestId("wallet-switcher")).toHaveTextContent("Primary"));
    expect(screen.queryByTestId("network-strip")).toBeNull();
  });
});

describe("route aliases", () => {
  it.each(ROUTE_ALIASES.map((a) => [a.from, a.to]))("%s redirects to %s", (from, to) => {
    render(
      <Routes>
        {ROUTE_ALIASES.map((a) => (
          <Route key={a.from} path={a.from} element={<Navigate to={a.to} replace />} />
        ))}
        <Route path="*" element={<Where />} />
      </Routes>,
      { wrapper: wrap(from) },
    );
    expect(screen.getByTestId("where")).toHaveTextContent(to);
  });
});
