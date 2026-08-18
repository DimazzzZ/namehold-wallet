import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn(),
}));

import { ActivityView } from "../ActivityView";

const profile = {
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  // Mainnet so name/txid/height render as clickable Shakeshift explorer
  // links (buttons). Tests here exercise rendering/filtering, not network
  // gating; the plain-text (non-mainnet) branch is covered manually.
  network: "mainnet",
  accountXpub: "xpubFAKE",
  accountIndex: 0,
  receiveDepth: 20,
  changeDepth: 20,
  receiveAddress: "rs1qexamplereceiveaddr",
  lastSyncedHeight: 10,
  lastSyncedAt: null,
  watchOnly: false,
  hasPassphrase: true,
  active: true,
};

function row(overrides: Record<string, unknown>) {
  return {
    txid: "aa",
    action: "receive",
    name: null,
    nameHash: null,
    valueDoos: 100_000_000,
    direction: "receive",
    height: 100,
    time: 1_700_000_000,
    confirmed: true,
    counterparty: null,
    ...overrides,
  };
}

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/activity"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

/** Route the base commands ActivityView + useActiveProfile need. */
function routes(historyResult: unknown) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "read_action_history":
        if (historyResult instanceof Error) return Promise.reject(historyResult);
        return Promise.resolve(historyResult);
      default:
        return Promise.resolve(null);
    }
  };
}

beforeEach(() => {
  invokeMock.mockReset();
});

describe("ActivityView", () => {
  it("renders classified rows with action badges", async () => {
    invokeMock.mockImplementation(
      routes([
        row({ txid: "aa", action: "receive", valueDoos: 100_000_000, direction: "receive" }),
        row({
          txid: "bb",
          action: "bid",
          name: "foo",
          nameHash: "deadbeef",
          // BID self-homes onto our own address — new backend semantics
          // report 0 (matches `netSpendDoos` for the drafts card).
          valueDoos: 0,
          direction: "send",
          height: 200,
        }),
      ]),
    );
    render(<ActivityView />, { wrapper: wrapper() });

    // Wait for actual table rows (the ".foo" cell), not just filter-dropdown
    // labels that share the "Bid"/"Receive" text.
    await waitFor(() =>
      expect(
        screen.getByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".foo"),
      ).toBeInTheDocument(),
    );
    // Row badges: only the row renders these inside a <span> Badge; the
    // filter dropdown uses <option>.
    const badges = screen.getAllByText("Bid");
    expect(badges.some((el) => el.tagName === "SPAN")).toBe(true);
    const rec = screen.getAllByText("Receive");
    expect(rec.some((el) => el.tagName === "SPAN")).toBe(true);
    // Alignment: BID amount cell shows "0.000000", byte-identical to the
    // drafts card's `formatHns(netSpendDoos(...))` for a self-homed action.
    expect(screen.getByText("0.000000")).toBeInTheDocument();
    // Color tone: zero-value BID (self-homed) must NOT be red. Receive
    // (positive net inflow) MUST be green.
    const zeroSpans = screen
      .getAllByText("0.000000")
      .filter((el) => el.tagName === "SPAN");
    expect(zeroSpans.length).toBeGreaterThan(0);
    expect(zeroSpans.some((el) => el.className.includes("text-gray-700"))).toBe(true);
    expect(zeroSpans.some((el) => el.className.includes("text-red-600"))).toBe(false);
    const receiveSpan = screen
      .getAllByText((_, el) => el?.tagName === "SPAN" && /^\+100\.000000$/.test(el.textContent ?? ""))
      .find((el) => el.className.includes("text-green-600"));
    expect(receiveSpan).toBeTruthy();
  });

  it("filters by action via the action select", async () => {
    invokeMock.mockImplementation(
      routes([
        row({ txid: "aa", action: "receive", direction: "receive" }),
        row({ txid: "bb", action: "bid", name: "foo", direction: "send", height: 200 }),
      ]),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    // Wait for the row's badge (span), not the dropdown option.
    await waitFor(() => {
      const nodes = screen.getAllByText("Receive");
      expect(nodes.some((el) => el.tagName === "SPAN")).toBe(true);
    });

    // The first combobox is the action filter.
    const selects = screen.getAllByRole("combobox");
    fireEvent.change(selects[0]!, { target: { value: "bid" } });

    // After filtering, no row-badge for Receive remains (dropdown option still exists).
    await waitFor(() => {
      const nodes = screen.queryAllByText("Receive");
      expect(nodes.some((el) => el.tagName === "SPAN")).toBe(false);
    });
    const bidBadges = screen.getAllByText("Bid");
    expect(bidBadges.some((el) => el.tagName === "SPAN")).toBe(true);
  });

  it("filters by name search", async () => {
    invokeMock.mockImplementation(
      routes([
        row({ txid: "aa", action: "bid", name: "foo", direction: "send" }),
        row({ txid: "bb", action: "bid", name: "bar", direction: "send" }),
      ]),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(
        screen.getByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".foo"),
      ).toBeInTheDocument(),
    );

    fireEvent.change(screen.getByPlaceholderText("Search by name..."), {
      target: { value: "foo" },
    });
    await waitFor(() =>
      expect(
        screen.queryByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".bar"),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".foo"),
    ).toBeInTheDocument();
  });

  it("shows the index-disabled banner when the node lacks the address index", async () => {
    invokeMock.mockImplementation(
      routes(new Error("address index not enabled on this hsd node: ... (status 400)")),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(
        screen.getByText(/Your node does not have/),
      ).toBeInTheDocument(),
    );
  });

  it("shows an empty state when there is no activity", async () => {
    invokeMock.mockImplementation(routes([]));
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(screen.getByText("No activity yet on this wallet.")).toBeInTheDocument(),
    );
  });

  it("renders dates in long form with time (e.g. 'July 24, 2026 - 12:00:00')", async () => {
    // Unix seconds for 2026-07-24 12:00:00 UTC.
    const unix = Math.floor(Date.UTC(2026, 6, 24, 12) / 1000);
    invokeMock.mockImplementation(
      routes([row({ txid: "cc", time: unix, height: 500 })]),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(screen.getByText("July 24, 2026 - 12:00:00")).toBeInTheDocument(),
    );
    // And the compact "24/07/2026, 12:00:00" locale-string form is absent.
    expect(screen.queryByText(/^\d{1,2}\/\d{1,2}\/\d{4}/)).not.toBeInTheDocument();
  });

  it("paginates when there are more than 50 rows", async () => {
    // 75 rows of confirmed receives at descending heights so ordering is
    // stable (backend already sorts newest-first, we preserve).
    const many = Array.from({ length: 75 }, (_, i) =>
      row({
        txid: `tx${String(i).padStart(3, "0")}`,
        action: "receive",
        direction: "receive",
        valueDoos: 1_000_000,
        height: 1000 - i, // 1000, 999, 998, …
      }),
    );
    invokeMock.mockImplementation(routes(many));
    render(<ActivityView />, { wrapper: wrapper() });
    // Wait for the table to appear.
    await waitFor(() => expect(screen.getByText(/Rows 1–50 of 75/)).toBeInTheDocument());
    // First page: 50 rows in <tbody>.
    const tbodyRowsPage1 = document.querySelectorAll("tbody tr");
    expect(tbodyRowsPage1.length).toBe(50);
    // Prev disabled on page 1; Next enabled.
    const prev = screen.getByRole("button", { name: /Prev/ });
    const next = screen.getByRole("button", { name: /Next/ });
    expect(prev).toBeDisabled();
    expect(next).not.toBeDisabled();

    fireEvent.click(next);
    await waitFor(() => expect(screen.getByText(/Rows 51–75 of 75/)).toBeInTheDocument());
    // Second page: only 25 rows remain.
    const tbodyRowsPage2 = document.querySelectorAll("tbody tr");
    expect(tbodyRowsPage2.length).toBe(25);
    // Prev now enabled; Next now disabled.
    expect(screen.getByRole("button", { name: /Prev/ })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: /Next/ })).toBeDisabled();
  });

  it("resets to page 1 when filters shrink the list past the current page", async () => {
    // 60 rows: 55 receives (heights 100..46) + 5 bids (heights 45..41).
    // On page 2 the user sees rows 51..60 (mostly bids at the end).
    const many = [
      ...Array.from({ length: 55 }, (_, i) =>
        row({
          txid: `rx${String(i).padStart(3, "0")}`,
          action: "receive",
          direction: "receive",
          valueDoos: 1_000_000,
          height: 100 - i,
        }),
      ),
      ...Array.from({ length: 5 }, (_, i) =>
        row({
          txid: `bx${String(i).padStart(3, "0")}`,
          action: "bid",
          direction: "send",
          valueDoos: 0,
          name: "foo",
          height: 45 - i,
        }),
      ),
    ];
    invokeMock.mockImplementation(routes(many));
    render(<ActivityView />, { wrapper: wrapper() });
    // Land on page 1 first.
    await waitFor(() => expect(screen.getByText(/Rows 1–50 of 60/)).toBeInTheDocument());
    // Click Next → page 2 (rows 51–60).
    fireEvent.click(screen.getByRole("button", { name: /Next/ }));
    await waitFor(() => expect(screen.getByText(/Rows 51–60 of 60/)).toBeInTheDocument());

    // Filter to "bid" → only 5 rows total. Should snap back to page 1 and
    // the pager should disappear (5 <= PAGE_SIZE).
    const [actionSelect] = screen.getAllByRole("combobox");
    fireEvent.change(actionSelect!, { target: { value: "bid" } });
    await waitFor(() => {
      // Pager hidden when the filtered list fits on one page.
      expect(screen.queryByText(/Rows \d+–\d+ of \d+/)).not.toBeInTheDocument();
    });
    // And 5 bid rows render.
    const rows5 = document.querySelectorAll("tbody tr");
    expect(rows5.length).toBe(5);
  });
});

describe("ActivityView — canonical table design", () => {
  it("the Activity table follows the unified table contract", async () => {
    invokeMock.mockImplementation(
      routes([
        row({ txid: "aa", action: "receive", valueDoos: 100_000_000, direction: "receive" }),
        row({
          txid: "bb",
          action: "bid",
          name: "foo",
          nameHash: "deadbeef",
          valueDoos: 0,
          direction: "send",
          height: 200,
        }),
      ]),
    );
    render(<ActivityView />, { wrapper: wrapper() });

    await waitFor(() =>
      expect(
        screen.getByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".foo"),
      ).toBeInTheDocument(),
    );

    const { assertCanonicalTable } = await import("../../test/canonicalTable");
    // ActivityView renders a single table.
    const tables = document.querySelectorAll("table");
    expect(tables.length).toBe(1);
    assertCanonicalTable(tables[0] as HTMLTableElement, { name: "Activity" });
  });

  it("batch draft row: composite label is non-clickable; individual names are reachable via safe links", async () => {
    // A batch-bid draft with 2 names. Backend persists the synthetic label
    // "js + 1 more" as `name`, and the true list as `nameList`. The activity
    // row shows the composite as a collapsed toggle; clicking expands into
    // clickable individual names, and clicking one opens NameInfoModal.
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "read_action_history":
          return Promise.resolve([]);
        case "list_tx_drafts":
          return Promise.resolve([
            {
              id: "draft-batch-1",
              walletProfileId: "p1",
              action: "batch-bid",
              status: "draft",
              summary: {
                action: "batch-bid",
                sendTotalDoos: 0,
                feeDoos: 5_000,
                changeDoos: 0,
                inputTotalDoos: 0,
                numInputs: 1,
                recipientAddress: null,
                txid: null,
                warnings: [],
                name: "js + 1 more",
                nameList: ["js", "c"],
              },
              errorMessage: null,
              txid: null,
              confirmationHeight: null,
              createdAt: "2026-08-14 12:00:00",
            },
          ]);
        default:
          return Promise.resolve(null);
      }
    });
    render(<ActivityView />, { wrapper: wrapper() });

    // Composite label renders as a toggle button (not a name-info link).
    const summaryToggle = await screen.findByTestId("activity-batch-summary-toggle");
    expect(summaryToggle.textContent).toContain("+ 1 more");

    // Individual names MUST NOT be visible by default (collapsed).
    expect(
      screen.queryByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".js"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".c"),
    ).not.toBeInTheDocument();

    // Click the summary to expand.
    fireEvent.click(summaryToggle);

    // Individual name links MUST now be visible as real buttons.
    await waitFor(() =>
      expect(
        screen.getByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".js"),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByText((_, el) => el?.tagName === "BUTTON" && el.textContent === ".c"),
    ).toBeInTheDocument();

    // The composite toggle itself never routes through onNameClick — it
    // has no data-testid="activity-name-info-link". Only the expanded
    // per-name buttons do.
    const infoLinks = screen.getAllByTestId("activity-name-info-link");
    expect(infoLinks).toHaveLength(2);
  });
});

describe("ActivityView — inline draft actions", () => {
  /** Build a single-draft `list_tx_drafts` payload with the given status. */
  function draftPayload(status: string, overrides: Record<string, unknown> = {}) {
    return [
      {
        id: "draft-act-1",
        walletProfileId: "p1",
        action: "send_hns",
        status,
        summary: {
          action: "send_hns",
          sendTotalDoos: 1_000_000,
          feeDoos: 5_000,
          changeDoos: 0,
          inputTotalDoos: 1_005_000,
          numInputs: 1,
          recipientAddress: "rs1qrecipient",
          txid: null,
          warnings: [],
          name: null,
          nameList: null,
        },
        errorMessage: null,
        txid: null,
        confirmationHeight: null,
        createdAt: "2026-08-14 12:00:00",
        ...overrides,
      },
    ];
  }

  function routeDrafts(status: string, extra?: (cmd: string) => unknown) {
    return (cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "read_action_history":
          return Promise.resolve([]);
        case "list_tx_drafts":
          return Promise.resolve(draftPayload(status));
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: "p1",
            unlocked: true,
            unlockedUntilEpochMs: Date.now() + 3_600_000,
          });
        default:
          return extra ? Promise.resolve(extra(cmd)) : Promise.resolve(null);
      }
    };
  }

  it("draft row shows Sign & broadcast + Discard; broadcasted row shows neither", async () => {
    invokeMock.mockImplementation(routeDrafts("draft"));
    render(<ActivityView />, { wrapper: wrapper() });

    const execBtn = await screen.findByTestId("activity-draft-execute");
    expect(execBtn.textContent).toBe("Sign & broadcast");
    expect(screen.getByTestId("activity-draft-discard")).toBeInTheDocument();
  });

  it("broadcasted row offers no draft actions", async () => {
    invokeMock.mockImplementation(routeDrafts("broadcasted"));
    render(<ActivityView />, { wrapper: wrapper() });
    // Wait for the table to render (fee cell present).
    await waitFor(() =>
      expect(screen.getByText("Pending")).toBeInTheDocument(),
    );
    expect(screen.queryByTestId("activity-draft-execute")).not.toBeInTheDocument();
    expect(screen.queryByTestId("activity-draft-discard")).not.toBeInTheDocument();
  });

  it("failed row shows Retry + Discard", async () => {
    invokeMock.mockImplementation(routeDrafts("failed"));
    render(<ActivityView />, { wrapper: wrapper() });
    const execBtn = await screen.findByTestId("activity-draft-execute");
    expect(execBtn.textContent).toBe("Retry");
    expect(screen.getByTestId("activity-draft-discard")).toBeInTheDocument();
  });

  it("clicking Sign & broadcast triggers sign_tx_draft then broadcast_tx_draft", async () => {
    const calls: string[] = [];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "sign_tx_draft" || cmd === "broadcast_tx_draft") calls.push(cmd);
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "read_action_history":
          return Promise.resolve([]);
        case "list_tx_drafts":
          return Promise.resolve(draftPayload("draft"));
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 3_600_000 });
        case "sign_tx_draft":
          return Promise.resolve({ id: "draft-act-1", status: "signed" });
        case "broadcast_tx_draft":
          return Promise.resolve({ draftId: "draft-act-1", txid: "abcdef012345", status: "broadcasted" });
        default:
          return Promise.resolve(null);
      }
    });
    render(<ActivityView />, { wrapper: wrapper() });
    const execBtn = await screen.findByTestId("activity-draft-execute");
    fireEvent.click(execBtn);
    await waitFor(() => {
      expect(calls).toEqual(["sign_tx_draft", "broadcast_tx_draft"]);
    });
  });

  it("clicking Discard (confirmed) triggers delete_tx_draft", async () => {
    const origConfirm = window.confirm;
    window.confirm = vi.fn(() => true);
    const calls: string[] = [];
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === "delete_tx_draft") calls.push(cmd);
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "read_action_history":
          return Promise.resolve([]);
        case "list_tx_drafts":
          return Promise.resolve(draftPayload("draft"));
        case "get_signer_session":
          return Promise.resolve({ walletProfileId: "p1", unlocked: true, unlockedUntilEpochMs: Date.now() + 3_600_000 });
        case "delete_tx_draft":
          return Promise.resolve(null);
        default:
          return Promise.resolve(null);
      }
    });
    render(<ActivityView />, { wrapper: wrapper() });
    const discardBtn = await screen.findByTestId("activity-draft-discard");
    fireEvent.click(discardBtn);
    await waitFor(() => {
      expect(calls).toEqual(["delete_tx_draft"]);
    });
    window.confirm = origConfirm;
  });

  it("Txid cell for a draft row renders a span, not a button (regression guard)", async () => {
    invokeMock.mockImplementation(routeDrafts("draft"));
    render(<ActivityView />, { wrapper: wrapper() });
    await screen.findByTestId("activity-draft-execute");
    // No tx-info-link button exists for a draft with null txid.
    expect(screen.queryByTestId("activity-tx-info-link")).not.toBeInTheDocument();
  });
});
