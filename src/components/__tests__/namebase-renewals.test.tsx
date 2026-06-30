import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, within, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// The "Expiring soon" panel surfaces Namebase's renewal calendar
// (/api/domains/renewals) so a migrating user can renew/move a custodial domain
// before it lapses — sorted soonest-first, with auto-renew risk flagged.

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NamebaseDashboard } from "../NamebaseDashboard";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "mainnet",
  accountXpub: "xpubFAKE",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "hs1q79vn7nsmua98v4gme98w0a07rgrvvxy9d93qw8",
  lastSyncedHeight: 10,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

function route(opts: { connected?: boolean; renewals?: unknown[] } = {}) {
  const connected = opts.connected ?? true;
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_namebase_status":
        return Promise.resolve({
          connected,
          has_cookie: connected,
          account: connected
            ? { balance: { hns: 100, btc: 0 }, pendingHns: 0, has2fa: false, withdrawalFeeHns: 1, minimums: { hns: 1 } }
            : undefined,
        });
      case "fetch_namebase_domains":
        return Promise.resolve({
          domains: [
            { name: "soon", owner_id: "o", owned_since: "2024-01-01", auto_renew_active: false, status: "active" },
            { name: "later", owner_id: "o", owned_since: "2024-01-01", auto_renew_active: true, status: "active" },
          ],
        });
      case "fetch_namebase_staked":
        return Promise.resolve({ stakedDomains: [] });
      case "fetch_namebase_renewals":
        // Returned OUT of order on purpose; the panel must sort soonest-first.
        return Promise.resolve({
          expiring: opts.renewals ?? [
            { domain: "later", expire_block: 340000, estimated_date: "2026-09-01T00:00:00.000Z" },
            { domain: "soon", expire_block: 339000, estimated_date: "2026-07-05T00:00:00.000Z" },
          ],
        });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("NamebaseDashboard — Expiring soon panel", () => {
  it("lists expiring domains soonest-first with real dates and auto-renew risk", async () => {
    invokeMock.mockImplementation(route());
    render(<NamebaseDashboard />, { wrapper: wrapper() });

    const panel = await screen.findByTestId("namebase-expiring");
    expect(panel).toHaveTextContent(/Expiring soon \(2\)/);

    // Sorted soonest-first: ".soon" (Jul) before ".later" (Sep), despite the
    // backend returning them in the opposite order.
    const names = within(panel)
      .getAllByText(/^\.(soon|later)$/)
      .map((el) => el.textContent);
    expect(names).toEqual([".soon", ".later"]);

    // Dates render (tz-aware formatDate) — never "Invalid Date".
    expect(within(panel).queryByText(/Invalid Date/i)).toBeNull();
    expect(within(panel).getAllByText(/2026/).length).toBeGreaterThan(0);

    // The auto-renew-off domain is flagged as highest risk.
    const soonRow = within(panel).getByText(".soon").closest("tr")!;
    expect(within(soonRow).getByText("Off")).toBeInTheDocument();
    const laterRow = within(panel).getByText(".later").closest("tr")!;
    expect(within(laterRow).getByText("On")).toBeInTheDocument();
  });

  it("is hidden when there are no expiring domains", async () => {
    invokeMock.mockImplementation(route({ renewals: [] }));
    render(<NamebaseDashboard />, { wrapper: wrapper() });
    await screen.findByText(/Your Domains/i);
    expect(screen.queryByTestId("namebase-expiring")).toBeNull();
  });

  it("does not query renewals when disconnected (panel hidden)", async () => {
    invokeMock.mockImplementation(route({ connected: false }));
    render(<NamebaseDashboard />, { wrapper: wrapper() });
    await screen.findByText(/Connect to Namebase/i);
    expect(screen.queryByTestId("namebase-expiring")).toBeNull();
    await waitFor(() => {
      expect(invokeMock.mock.calls.map((c) => c[0])).not.toContain("fetch_namebase_renewals");
    });
  });
});
