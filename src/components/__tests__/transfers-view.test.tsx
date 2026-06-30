import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { TransfersView } from "../TransfersView";

const profile = {
  id: "p1",
  label: "Primary",
  network: "mainnet",
  receiveAddress: "hs1qmywallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

function routeInvoke(opts: { domains?: unknown[]; withdrawals?: unknown[] }) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "fetch_namebase_domain_withdrawals":
        return Promise.resolve({ withdrawals: opts.domains ?? [] });
      case "fetch_namebase_withdrawals":
        return Promise.resolve({ withdrawals: opts.withdrawals ?? [] });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => invokeMock.mockReset());

describe("TransfersView", () => {
  it("mirrors Namebase's domain-transfer statuses", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        domains: [
          {
            id: "d1",
            domain: "exampletld",
            destination_address: "hs1qmywallet",
            status: "transfer_completed",
            status_note: null,
            created_at: "2026-06-26T00:00:00Z",
            updated_at: "2026-06-27T00:00:00Z",
          },
          {
            id: "d2",
            domain: "examplename",
            destination_address: "hs1qdestaddr",
            status: "finalize_completed",
            status_note: null,
            created_at: "2026-06-20T00:00:00Z",
            updated_at: "2026-06-22T00:00:00Z",
          },
        ],
      }),
    );
    render(<TransfersView />, { wrapper: wrapper() });

    // Both domains appear with Namebase's own (humanized) statuses.
    expect(await screen.findByText(/\.exampletld/)).toBeInTheDocument();
    expect(await screen.findByText(/Transfer sent — finalizing/i)).toBeInTheDocument();
    expect(await screen.findByText(/\.examplename/)).toBeInTheDocument();
    expect(await screen.findByText(/^Completed$/i)).toBeInTheDocument();
  });

  it("shows an HNS withdrawal with its Namebase status", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        withdrawals: [
          {
            id: "w1",
            currency: "hns",
            amount: "2000000", // 2 HNS in doos
            destination_address: "hs1qmywallet",
            status: "completed",
            status_note: null,
            created_at: "2026-06-26T13:23:32.000Z",
          },
        ],
      }),
    );
    render(<TransfersView />, { wrapper: wrapper() });

    expect(await screen.findByText(/HNS withdrawals/i)).toBeInTheDocument();
    expect(await screen.findByText(/^Completed$/i)).toBeInTheDocument();
    expect(screen.getByText(/2(\.0+)?\s*HNS/i)).toBeInTheDocument();
  });

  it("empty state when there are no transfers", async () => {
    invokeMock.mockImplementation(routeInvoke({}));
    render(<TransfersView />, { wrapper: wrapper() });
    expect(await screen.findByText(/No transfers yet/i)).toBeInTheDocument();
  });
});
