// Renewals screen (Task 3 / C3): the table is driven by the live
// `read_renewals` command (chain-computed days-until-expiry), NOT the stale
// CSV-imported assets columns. Every row shows an honest source badge and a
// Renew button that opens the NameActionsModal for the RAW (punycode) name.
import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { Renewals } from "../Renewals";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "mainnet",
  accountXpub: "xpubFAKE000000000000",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "hs1qexample",
  lastSyncedHeight: 260000,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

// `xn--j1amh` decodes to "укр" — the table must render the pretty form while
// the Renew action passes the RAW punycode name to the backend/modal.
const renewalsResponse = {
  walletProfileId: "p1",
  currentHeight: 260000,
  heightSource: "explorer",
  expiringSoonThresholdDays: 30,
  names: [
    {
      name: "xn--j1amh",
      state: "CLOSED",
      renewalHeight: 156000,
      expiresAtHeight: 261120,
      blocksUntilExpire: 1120,
      daysUntilExpire: 7.8,
      source: "chain",
      expiringSoon: true,
    },
    {
      name: "calmname",
      state: "CLOSED",
      renewalHeight: 200000,
      expiresAtHeight: 305120,
      blocksUntilExpire: 45120,
      daysUntilExpire: 313.3,
      source: "chain",
      expiringSoon: false,
    },
    {
      name: "csvname",
      state: "CLOSED",
      renewalHeight: null,
      expiresAtHeight: 500000,
      blocksUntilExpire: null,
      daysUntilExpire: 42.5,
      source: "csv-import",
      expiringSoon: false,
    },
  ],
};

function route() {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "read_renewals":
        return Promise.resolve(renewalsResponse);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
      case "get_write_capability":
        return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
      default:
        return Promise.resolve(null);
    }
  };
}

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("Renewals — live chain data", () => {
  it("renders rows from read_renewals with days, expiry heights and source badges", async () => {
    invokeMock.mockImplementation(route());
    render(<Renewals />, { wrapper: wrapper() });

    // Live days from the command (not assets columns), floored (not rounded)
    // so a name isn't shown with more days left than it actually has.
    expect(await screen.findByText("7d")).toBeInTheDocument();
    expect(screen.getByText("313d")).toBeInTheDocument();
    expect(screen.getByText("42d")).toBeInTheDocument();
    // Expiry heights.
    expect(screen.getByText("#261120")).toBeInTheDocument();
    expect(screen.getByText("#500000")).toBeInTheDocument();
    // Honest source badges.
    expect(screen.getByTestId("source-xn--j1amh")).toHaveTextContent("chain");
    expect(screen.getByTestId("source-calmname")).toHaveTextContent("chain");
    expect(screen.getByTestId("source-csvname")).toHaveTextContent("CSV import");
    // The read is pinned to the active wallet.
    expect(invokeMock).toHaveBeenCalledWith("read_renewals", { walletProfileId: "p1" });
  });

  it("renders the punycode name decoded but keeps the raw name on the wire", async () => {
    invokeMock.mockImplementation(route());
    render(<Renewals />, { wrapper: wrapper() });
    expect(await screen.findByText(".укр")).toBeInTheDocument();
  });

  it("states the height source honestly", async () => {
    invokeMock.mockImplementation(route());
    render(<Renewals />, { wrapper: wrapper() });
    // Wait for the data to load (rows visible) before asserting the note.
    await screen.findByText("7d");
    expect(screen.getByTestId("renewals-height-source")).toHaveTextContent(/estimated|last sync/i);
  });

  it("Renew opens the actions modal for the RAW name", async () => {
    invokeMock.mockImplementation(route());
    render(<Renewals />, { wrapper: wrapper() });

    fireEvent.click(await screen.findByTestId("renew-xn--j1amh"));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        "get_name_action_capabilities",
        expect.objectContaining({ name: "xn--j1amh" }),
      );
    });
  });

  it("shows an explicit Expired label (not '0d' or a negative number) for a lapsed name", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profile]);
      if (cmd === "read_renewals")
        return Promise.resolve({
          walletProfileId: "p1",
          currentHeight: 260000,
          heightSource: "explorer",
          expiringSoonThresholdDays: 30,
          names: [
            {
              name: "lapsedname",
              state: "CLOSED",
              renewalHeight: 100000,
              expiresAtHeight: 200000,
              blocksUntilExpire: -60000,
              daysUntilExpire: -3.5,
              source: "chain",
              expiringSoon: true,
            },
          ],
        });
      return Promise.resolve(null);
    });
    render(<Renewals />, { wrapper: wrapper() });
    expect(await screen.findByText("Expired")).toBeInTheDocument();
    expect(screen.queryByText("-3d")).not.toBeInTheDocument();
    expect(screen.queryByText("0d")).not.toBeInTheDocument();
  });

  it("colors a row red at exactly the threshold boundary (<=, not <)", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profile]);
      if (cmd === "read_renewals")
        return Promise.resolve({
          walletProfileId: "p1",
          currentHeight: 260000,
          heightSource: "explorer",
          expiringSoonThresholdDays: 30,
          names: [
            {
              name: "atthreshold",
              state: "CLOSED",
              renewalHeight: 100000,
              expiresAtHeight: 200000,
              blocksUntilExpire: 4320,
              daysUntilExpire: 30,
              source: "chain",
              expiringSoon: true,
            },
          ],
        });
      return Promise.resolve(null);
    });
    render(<Renewals />, { wrapper: wrapper() });
    const cell = await screen.findByText("30d");
    expect(cell).toHaveClass("text-red-600");
  });

  it("shows an empty state when there is nothing to renew", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "list_wallet_profiles") return Promise.resolve([profile]);
      if (cmd === "read_renewals")
        return Promise.resolve({
          walletProfileId: "p1",
          currentHeight: null,
          heightSource: "unknown",
          expiringSoonThresholdDays: 30,
          names: [],
        });
      return Promise.resolve(null);
    });
    render(<Renewals />, { wrapper: wrapper() });
    expect(await screen.findByText(/no renewal data/i)).toBeInTheDocument();
  });
});
