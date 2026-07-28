/**
 * Canonical table design assertions for components that previously only had
 * smoke tests: Batches (detail table) and TldInventory (DataTable, virtualized).
 *
 * Both fetch data through `../../lib/invoke` (not `@tauri-apps/api/core`
 * directly), so we mock that single seam.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";
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

import { Batches } from "../Batches";
import { TldInventory } from "../TldInventory";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "regtest",
  accountXpub: "xpubFAKE",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "rs1qexample",
  lastSyncedHeight: 10,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

const asset = (id: number, tld: string) => ({
  id,
  tld,
  status: "not_started",
  is_staked: false,
  category: null,
  tags: [],
  notes: null,
  hns_received: null,
  transfer_tx_hash: null,
  finalize_tx_hash: null,
  name_state: null,
  expires_at_height: null,
  days_until_expire: null,
  last_synced_at: null,
  created_at: "2026-01-01",
  updated_at: "2026-01-01",
});

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

// `@tanstack/react-virtual` needs a measured scroll element to emit virtual
// rows. JSDOM reports 0 for every layout box, so without this stub the
// virtualized <tbody> renders no <tr> children. Give elements a realistic
// viewport so the (small) row set virtualizes fully.
beforeEach(() => {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
    width: 800,
    height: 600,
    top: 0,
    left: 0,
    right: 800,
    bottom: 600,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  } as DOMRect);
  // react-virtual reads `offsetHeight` on the scroll element on some paths.
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return 600;
    },
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Batches — canonical table design", () => {
  it("the batch-detail assets table follows the unified contract", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "list_batches":
          return Promise.resolve([
            { id: 1, name: "Batch 1", description: null, status: "planned", asset_count: 2, created_at: "2026-01-01", updated_at: "2026-01-01" },
          ]);
        case "get_batch_with_assets":
          return Promise.resolve({
            id: 1,
            name: "Batch 1",
            description: null,
            status: "planned",
            asset_count: 2,
            created_at: "2026-01-01",
            updated_at: "2026-01-01",
            assets: [asset(1, "foo"), asset(2, "bar")],
          });
        case "list_assets":
          return Promise.resolve([asset(1, "foo"), asset(2, "bar"), asset(3, "baz")]);
        default:
          return Promise.resolve(null);
      }
    });

    render(<Batches />, { wrapper: wrapper() });
    // Wait for the batch list to render, then click on the batch to open detail.
    const batchCard = await screen.findByText("Batch 1");
    fireEvent.click(batchCard);
    // Wait for the batch-detail table to render.
    await screen.findByText(".foo");

    const { assertCanonicalTable } = await import("../../test/canonicalTable");
    const table = document.querySelector("table");
    expect(table).toBeTruthy();
    assertCanonicalTable(table as HTMLTableElement, { name: "Batches-detail" });
  });
});

describe("TldInventory — canonical table design (virtualized DataTable)", () => {
  it("the DataTable follows the virtual-canonical contract", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
        case "get_write_capability":
          return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
        case "list_assets":
          return Promise.resolve([asset(1, "foo"), asset(2, "bar")]);
        default:
          return Promise.resolve(null);
      }
    });

    render(<TldInventory />, { wrapper: wrapper() });
    await screen.findByText(".foo");

    const { assertVirtualCanonicalTable } = await import("../../test/canonicalTable");
    const table = document.querySelector("table");
    expect(table).toBeTruthy();
    assertVirtualCanonicalTable(table as HTMLTableElement, { name: "TldInventory" });
  });
});
