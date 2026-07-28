import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

// Manage DNS: the DNS editor must seed from `read_name_records` and UPDATE must
// send the correct subset (or `[]` for delete-all).

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { NameActionsModal } from "../NameActionsModal";
import { useUiStore } from "../../stores/ui";

const profile = {
  id: "p1",
  label: "Primary",
  network: "regtest",
  receiveAddress: "rs1qwallet",
  watchOnly: false,
  hasPassphrase: false,
  active: true,
};

const ownedCaps = {
  name: "myname",
  phase: "CLOSED",
  taskState: "ownedNoUrgentAction",
  ownsName: true,
  hasBidCommitment: false,
  hasRevealCoin: false,
  hasOwnerCoin: true,
  canOpen: { allowed: false, reason: "Already registered" },
  canBid: { allowed: false, reason: "Already registered" },
  canReveal: { allowed: false, reason: null },
  canRedeem: { allowed: false, reason: null },
  canRegister: { allowed: true, reason: null },
  canUpdate: { allowed: true, reason: null },
  canTransfer: { allowed: true, reason: null },
  canFinalize: { allowed: false, reason: null },
  canCancelTransfer: { allowed: false, reason: null },
  canRenew: { allowed: true, reason: null },
  canRevoke: { allowed: true, reason: null },
  nextActionKey: null,
  nextActionLabel: null,
  nextActionReason: null,
  countdownLabel: null,
  countdownBlocks: null,
  countdownHours: null,
};

const currentRecords = [
  { type: "NS", ns: "ns1.example." },
  { type: "DS", keyTag: 12345, algorithm: 8, digestType: 2, digest: "ABCDEF01" },
];

const currentResource = {
  records: currentRecords,
};

function route(overrides: Record<string, (...args: unknown[]) => Promise<unknown>> = {}) {
  return (cmd: string, ...rest: unknown[]) => {
    if (overrides[cmd]) return overrides[cmd]!(...rest);
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "get_signer_session":
        return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 60000 });
      case "get_write_capability":
        return Promise.resolve({ signerUnlocked: true, broadcasterAvailable: true, canWrite: true, reason: null });
      case "read_name_info":
        return Promise.resolve({
          name: "myname",
          state: "CLOSED",
          registered: true,
          height: 100,
          renewal: 200,
          owner: { hash: "abc" },
          value: 1_000_000,
          highest: 1_000_000,
          stats: null,
        });
      case "get_name_action_capabilities":
        return Promise.resolve(ownedCaps);
      case "read_name_records":
        return Promise.resolve(currentResource);
      case "read_name_bids":
        return Promise.resolve({ name: "myname", state: null, highest: null, value: null, bids: [], myBidCount: 0 });
      case "build_update_draft":
        return Promise.resolve({ id: "draft-u1" });
      case "sign_tx_draft":
        return Promise.resolve({ id: "draft-u1" });
      case "broadcast_tx_draft":
        return Promise.resolve({ txid: "abcdef0123456789" });
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

beforeEach(() => {
  invokeMock.mockReset();
  useUiStore.setState({ toastQueue: [], toastMessage: null });
});

describe("NameActionsModal — DNS records prefill (Manage DNS)", () => {
  it("seeds the editor with current records from read_name_records", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="myname" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Wait for the DNS section to appear (owned names auto-expand management).
    const rows = await screen.findByTestId("dns-rows-advanced");
    // Should have 2 rows (NS + DS) seeded from the mock records.
    const selects = rows.querySelectorAll("select");
    expect(selects).toHaveLength(2);
    expect((selects[0] as HTMLSelectElement).value).toBe("NS");
    expect((selects[1] as HTMLSelectElement).value).toBe("DS");

    // NS row has the value populated.
    const nsInput = rows.querySelectorAll('[aria-label="record value"]');
    expect(nsInput[0]).toHaveValue("ns1.example.");

    // DS row has its 4 fields populated as strings.
    expect(rows.querySelector('[aria-label="key tag"]')).toHaveValue("12345");
    expect(rows.querySelector('[aria-label="algorithm"]')).toHaveValue("8");
    expect(rows.querySelector('[aria-label="digest type"]')).toHaveValue("2");
    expect(rows.querySelector('[aria-label="digest"]')).toHaveValue("ABCDEF01");
  });

  it("passes the RAW name to read_name_records (not punycode-decoded)", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="myname" open onClose={vi.fn()} />, { wrapper: wrapper() });
    await screen.findByTestId("dns-rows-advanced");

    const calls = invokeMock.mock.calls.filter((c: unknown[]) => c[0] === "read_name_records");
    expect(calls.length).toBeGreaterThan(0);
    expect(calls[0]![1]).toMatchObject({ name: "myname" });
  });

  it("UPDATE sends remaining records after removing one row", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="myname" open onClose={vi.fn()} />, { wrapper: wrapper() });

    const rows = await screen.findByTestId("dns-rows-advanced");
    // Remove the first row (NS).
    const removeButtons = rows.querySelectorAll('[aria-label="remove record"]');
    fireEvent.click(removeButtons[0]!);

    // Click Update.
    const updateBtn = await screen.findByRole("button", { name: /^Update$/ });
    fireEvent.click(updateBtn);

    await waitFor(() => {
      const buildCalls = invokeMock.mock.calls.filter(
        (c: unknown[]) => c[0] === "build_update_draft",
      );
      expect(buildCalls.length).toBeGreaterThan(0);
      const records = (buildCalls[0]![1] as { records: Array<Record<string, unknown>> }).records;
      // Only the DS record remains.
      expect(records).toHaveLength(1);
      expect(records[0]!.type).toBe("DS");
      expect(records[0]!.keyTag).toBe(12345);
    });
  });

  it("UPDATE sends [] when all records are removed (delete-all)", async () => {
    invokeMock.mockImplementation(route());
    render(<NameActionsModal name="myname" open onClose={vi.fn()} />, { wrapper: wrapper() });

    const rows = await screen.findByTestId("dns-rows-advanced");
    // Remove the first row (NS); the second (DS) becomes the only row.
    const removeButtons = rows.querySelectorAll('[aria-label="remove record"]');
    fireEvent.click(removeButtons[0]!);

    // Clear the remaining DS row's required fields so it serializes to null
    // (rowToRecord returns null when any DS field is blank).
    const keyTagInput = rows.querySelector('[aria-label="key tag"]') as HTMLInputElement;
    fireEvent.change(keyTagInput, { target: { value: "" } });

    // Click Update.
    const updateBtn = await screen.findByRole("button", { name: /^Update$/ });
    fireEvent.click(updateBtn);

    await waitFor(() => {
      const buildCalls = invokeMock.mock.calls.filter(
        (c: unknown[]) => c[0] === "build_update_draft",
      );
      expect(buildCalls.length).toBeGreaterThan(0);
      // Empty resource — `[]`, NOT `null`.
      expect((buildCalls[0]![1] as { records: unknown }).records).toEqual([]);
    });
  });

  it("shows the empty-records hint when the fresh read returns no records", async () => {
    invokeMock.mockImplementation(route({
      read_name_records: () => Promise.resolve({ records: [] }),
    }));
    render(<NameActionsModal name="myname" open onClose={vi.fn()} />, { wrapper: wrapper() });

    const hint = await screen.findByTestId("dns-records-hint");
    expect(hint).toBeInTheDocument();
    expect(hint.textContent).toContain("no records yet");
  });

  it("shows the stale banner and disables UPDATE when the fresh read errors", async () => {
    invokeMock.mockImplementation(route({
      read_name_records: () => Promise.reject(new Error("node not synced")),
    }));
    render(<NameActionsModal name="myname" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // The "can't read current records" banner appears...
    const banner = await screen.findByTestId("dns-records-stale-banner");
    expect(banner.textContent).toContain("Can't read");
    // ...and the UPDATE button is disabled (never render the editor either).
    const updateBtn = await screen.findByRole("button", { name: /^Update$/ });
    expect(updateBtn).toBeDisabled();
    // A Retry affordance is present.
    expect(screen.getByTestId("dns-records-retry")).toBeInTheDocument();
    // The row editor must NOT render from a non-fresh read.
    expect(screen.queryByTestId("dns-rows-advanced")).not.toBeInTheDocument();
  });

  it("enables UPDATE only once a fresh read has seeded the editor", async () => {
    invokeMock.mockImplementation(route()); // read resolves with currentRecords
    render(<NameActionsModal name="myname" open onClose={vi.fn()} />, { wrapper: wrapper() });

    // Editor renders (gated on fresh) and UPDATE becomes enabled.
    await screen.findByTestId("dns-rows-advanced");
    const updateBtn = await screen.findByRole("button", { name: /^Update$/ });
    expect(updateBtn).not.toBeDisabled();
  });
});
