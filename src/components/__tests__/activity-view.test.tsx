import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { LocationProbe } from "../../test/namePage";
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
function routes(historyResult: unknown, drafts: unknown[] = []) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "list_tx_drafts":
        return Promise.resolve(drafts);
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
  // The values are the draft actions of `sell.rs::{LOCK_ACTION,
  // LOCK_FINALIZE_ACTION}` and `cancel.rs::{CANCEL_ACTION,
  // CANCEL_FINALIZE_ACTION}`.
  it.each([
    ["shakedex_lock", "Lock for sale"],
    ["shakedex_lock_finalize", "Finalize into lock"],
    ["shakedex_cancel", "Cancel listing"],
    ["shakedex_cancel_finalize", "Finalize cancel"],
  ])("labels the selling draft %s as %s, in the filter and on its row", async (action, label) => {
    invokeMock.mockImplementation(
      routes([row({ txid: "cc", action, name: "dexsale", direction: "send", height: 300 })]),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    const option = await screen.findByRole("option", { name: label });
    expect(option).toHaveValue(action);
    await waitFor(() =>
      expect(screen.getAllByText(label).some((el) => el.tagName === "SPAN")).toBe(true),
    );
  });

  it("a Lock for sale row seen on chain still shows the name's value, not a spend", async () => {
    // The lock TRANSFER re-homes the name's 0.5 HNS onto our own coin: net 0.
    invokeMock.mockImplementation(
      routes(
        [
          row({
            txid: "cc",
            action: "transfer",
            name: "dexsale",
            valueDoos: 0,
            nameValueDoos: 500_000,
            direction: "send",
            height: 300,
          }),
        ],
        [
          {
            id: "d1",
            walletProfileId: "p1",
            action: "shakedex_lock",
            status: "confirmed",
            summary: null,
            errorMessage: null,
            txid: "cc",
            confirmationHeight: 300,
            purchaseLostReason: null,
            createdAt: "2026-10-14 12:00:00",
          },
        ],
      ),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(screen.getAllByText("Lock for sale").some((el) => el.tagName === "SPAN")).toBe(true),
    );
    expect(screen.getByTestId("activity-locked-value")).toHaveTextContent("0.5");
  });

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
    // Amounts are short in the list (exact on hover). A zero-value BID
    // (self-homed) is neutral, never red; a Receive (net inflow) is green.
    const amounts = screen.getAllByTestId("activity-amount");
    const zero = amounts.find((el) => el.textContent === "0")!;
    expect(zero.className).toContain("text-gray-500");
    expect(zero.className).not.toContain("text-red-600");
    const receive = amounts.find((el) => el.textContent === "+100")!;
    expect(receive.className).toContain("text-green-600");
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
    await waitFor(() => expect(screen.getByText(/Your node does not have/)).toBeInTheDocument());
  });

  // Tauri rejects `invoke` with the command's error as serialized, a plain
  // string (`AppError` serializes to its message), not an `Error`.
  it("shows the index-disabled banner for the backend's string rejection", async () => {
    const base = routes([]);
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "read_action_history"
        ? Promise.reject(
            "Node RPC error: address index not enabled on this hsd node: restart hsd with --index-address and --index-tx",
          )
        : base(cmd),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() => expect(screen.getByText(/Your node does not have/)).toBeInTheDocument());
    expect(screen.queryByText(/Failed to load activity/)).not.toBeInTheDocument();
  });

  // `read_action_history` names exactly the index the node lacks; the banner
  // names that flag, and its remedy still restarts hsd with both.
  it.each([
    [
      "Node RPC error: address index not enabled on this hsd node: restart hsd with --index-address and --index-tx",
      "Your node does not have --index-address enabled.",
    ],
    [
      "Node RPC error: transaction index not enabled on this hsd node: restart hsd with --index-address and --index-tx",
      "Your node does not have --index-tx enabled.",
    ],
    [
      "Node RPC error: address and transaction indexes not enabled on this hsd node: restart hsd with --index-address and --index-tx",
      "Your node does not have --index-address or --index-tx enabled.",
    ],
  ])("the index banner for %s names the missing flag", async (rejection, names) => {
    const base = routes([]);
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "read_action_history" ? Promise.reject(rejection) : base(cmd),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    const banner = await screen.findByTestId("activity-index-banner");
    expect(banner.textContent).toContain(names);
    expect(banner.textContent).toContain("Restart it with --index-address --index-tx");
    expect(screen.queryByText(/Failed to load activity/)).not.toBeInTheDocument();
  });

  it("shows the backend's string rejection in the generic error banner", async () => {
    const base = routes([]);
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "read_action_history" ? Promise.reject("HTTP error: connection refused") : base(cmd),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(
        screen.getByText("Failed to load activity: HTTP error: connection refused"),
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

  it("renders dates short in History and long in the expanded row", async () => {
    // Unix seconds for 2026-07-24 12:00:00 UTC.
    const unix = Math.floor(Date.UTC(2026, 6, 24, 12) / 1000);
    invokeMock.mockImplementation(routes([row({ txid: "cc", time: unix, height: 500 })]));
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() => expect(screen.getByText("Jul 24, 2026 · 12:00")).toBeInTheDocument());
    fireEvent.click(screen.getByTestId("activity-row-toggle"));
    expect(screen.getByTestId("activity-row-detail")).toHaveTextContent("July 24, 2026 - 12:00:00");
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
    // Newer disabled on page 1; Show older enabled.
    const prev = screen.getByRole("button", { name: /Newer/ });
    const next = screen.getByRole("button", { name: /Show older/ });
    expect(prev).toBeDisabled();
    expect(next).not.toBeDisabled();

    fireEvent.click(next);
    await waitFor(() => expect(screen.getByText(/Rows 51–75 of 75/)).toBeInTheDocument());
    // Second page: only 25 rows remain.
    const tbodyRowsPage2 = document.querySelectorAll("tbody tr");
    expect(tbodyRowsPage2.length).toBe(25);
    // Newer now enabled; Show older now disabled.
    expect(screen.getByRole("button", { name: /Newer/ })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: /Show older/ })).toBeDisabled();
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
    fireEvent.click(screen.getByRole("button", { name: /Show older/ }));
    await waitFor(() => expect(screen.getByText(/Rows 51–60 of 60/)).toBeInTheDocument());

    // Filter to "bid" → only 5 rows total. Should snap back to page 1 and
    // the pager should disappear (5 <= PAGE_SIZE).
    const [actionSelect] = screen.getAllByRole("combobox");
    fireEvent.change(actionSelect!, { target: { value: "bid" } });
    await waitFor(() => {
      // Back on page 1; the page buttons hide when the list fits on one page.
      expect(screen.getByText("Rows 1–5 of 5")).toBeInTheDocument();
    });
    expect(screen.queryByText(/Page \d+ of \d+/)).not.toBeInTheDocument();
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
    // clickable individual names, and clicking one opens that name's page.
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
              // Finished, so it lands in History, where batches unfold.
              status: "confirmed",
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
              purchaseLostReason: null,
              createdAt: "2026-08-14 12:00:00",
            },
          ]);
        default:
          return Promise.resolve(null);
      }
    });
    render(
      <>
        <ActivityView />
        <LocationProbe />
      </>,
      { wrapper: wrapper() },
    );

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

    fireEvent.click(infoLinks[1]!);
    await waitFor(() => expect(screen.getByTestId("location")).toHaveTextContent("/names/c"));
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
        purchaseLostReason: null,
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
    await waitFor(() => expect(screen.getByText("Pending")).toBeInTheDocument());
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
          return Promise.resolve({
            walletProfileId: "p1",
            unlocked: true,
            unlockedUntilEpochMs: Date.now() + 3_600_000,
          });
        case "sign_tx_draft":
          return Promise.resolve({ id: "draft-act-1", status: "signed" });
        case "broadcast_tx_draft":
          return Promise.resolve({
            draftId: "draft-act-1",
            txid: "abcdef012345",
            status: "broadcasted",
          });
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
          return Promise.resolve({
            walletProfileId: "p1",
            unlocked: true,
            unlockedUntilEpochMs: Date.now() + 3_600_000,
          });
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

  // Discarding a purchase draft deletes its purchase record, which Owned
  // Names ("purchase pending") and the market page read.
  it.each([
    ["shakedex_purchase", true],
    ["send_hns", false],
  ])("discarding a %s draft refreshes Owned Names: %s", async (action, refreshes) => {
    const origConfirm = window.confirm;
    window.confirm = vi.fn(() => true);
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "read_action_history":
          return Promise.resolve([]);
        case "list_tx_drafts":
          return Promise.resolve(draftPayload("draft", { action }));
        default:
          return Promise.resolve(null);
      }
    });
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/activity"]}>
          <ActivityView />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    fireEvent.click(await screen.findByTestId("activity-draft-discard"));
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "delete_tx_draft")).toBe(true),
    );
    // The shared draft hooks refresh the wallet on every discard.
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ queryKey: ["wallet"] }));
    if (refreshes) {
      expect(spy).toHaveBeenCalledWith({ queryKey: ["read"] });
      expect(spy).toHaveBeenCalledWith({ queryKey: ["shakedex"] });
    } else {
      expect(spy).not.toHaveBeenCalledWith({ queryKey: ["read"] });
    }
    window.confirm = origConfirm;
  });

  it("a purchase draft whose send fails from Activity refreshes Owned Names", async () => {
    invokeMock.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile]);
        case "read_action_history":
          return Promise.resolve([]);
        case "list_tx_drafts":
          return Promise.resolve(draftPayload("failed", { action: "shakedex_purchase" }));
        case "get_signer_session":
          return Promise.resolve({
            walletProfileId: "p1",
            unlocked: true,
            unlockedUntilEpochMs: Date.now() + 3_600_000,
          });
        case "sign_tx_draft":
          return Promise.resolve({ id: "draft-act-1", status: "signed" });
        case "broadcast_tx_draft":
          return Promise.reject("TX rejected: bad-txns-inputs-missingorspent");
        default:
          return Promise.resolve(null);
      }
    });
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    const spy = vi.spyOn(qc, "invalidateQueries");
    render(
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/activity"]}>
          <ActivityView />
        </MemoryRouter>
      </QueryClientProvider>,
    );
    fireEvent.click(await screen.findByTestId("activity-draft-execute"));
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ queryKey: ["read"] }));
    expect(spy).toHaveBeenCalledWith({ queryKey: ["shakedex"] });
  });

  it("Txid cell for a draft row renders a span, not a button (regression guard)", async () => {
    invokeMock.mockImplementation(routeDrafts("draft"));
    render(<ActivityView />, { wrapper: wrapper() });
    await screen.findByTestId("activity-draft-execute");
    // No tx-info-link button exists for a draft with null txid.
    expect(screen.queryByTestId("activity-tx-info-link")).not.toBeInTheDocument();
  });
});

describe("ActivityView: paid swap offers recorded before the withdrawal", () => {
  const offers = [
    {
      name: "newer",
      buyerAddress: "hs1qnewer",
      priceDoos: 2_500_000,
      transferTxid: "ab12",
      claimed: true,
      createdAt: "2026-09-10 10:00:00",
    },
    {
      name: "older",
      buyerAddress: "hs1qolder",
      priceDoos: 1_000_000,
      transferTxid: null,
      claimed: false,
      createdAt: "2026-09-01 10:00:00",
    },
  ];

  it("lists each recorded offer read-only, with its buyer, price and claim", async () => {
    const base = routes([]);
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_paid_swap_offers" ? Promise.resolve(offers) : base(cmd),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    const card = await screen.findByTestId("paid-swap-offers");
    expect(card).toHaveTextContent("Paid swap offers");
    const rows = within(card).getAllByTestId("paid-swap-offer");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent(".newer");
    expect(rows[0]).toHaveTextContent("hs1qnewer");
    expect(rows[0]).toHaveTextContent("2.5 HNS");
    expect(rows[0]).toHaveTextContent("Claimed");
    expect(rows[1]).toHaveTextContent("Not claimed");
    // Read-only: the names open the name, and nothing else is a control.
    expect(
      within(card)
        .getAllByRole("button")
        .map((b) => b.textContent),
    ).toEqual([".newer", ".older"]);
  });

  it("shows nothing when no offer was recorded", async () => {
    const base = routes([]);
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_paid_swap_offers" ? Promise.resolve([]) : base(cmd),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "list_paid_swap_offers")).toBe(true),
    );
    expect(screen.queryByTestId("paid-swap-offers")).toBeNull();
  });
});

describe("ActivityView — To finish / Waiting / History (v2)", () => {
  const draft = (id: string, status: string, over: Record<string, unknown> = {}) => ({
    id,
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
    purchaseLostReason: null,
    createdAt: "2026-08-14 12:00:00",
    ...over,
  });
  const world =
    (history: unknown, drafts: unknown[], extra: Record<string, unknown> = {}) =>
    (cmd: string) =>
      cmd in extra ? Promise.resolve(extra[cmd]) : routes(history, drafts)(cmd);

  it("splits rows into To finish → Waiting → History, in that order", async () => {
    invokeMock.mockImplementation(
      world(
        [row({ txid: "aa" }), row({ txid: "bb", action: "bid", name: "foo", confirmed: false })],
        [draft("d1", "draft"), draft("d2", "failed"), draft("d3", "broadcasted", { txid: "cc" })],
      ),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    const finish = await screen.findByTestId("activity-to-finish");
    const waiting = screen.getByTestId("activity-waiting");
    const history = screen.getByTestId("activity-history");
    expect(within(finish).getAllByTestId("activity-to-finish-row")).toHaveLength(2);
    expect(within(waiting).getAllByTestId("activity-waiting-row")).toHaveLength(2);
    expect(within(history).getAllByTestId("activity-history-row")).toHaveLength(1);
    expect(finish.compareDocumentPosition(waiting) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(
      waiting.compareDocumentPosition(history) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(within(finish).getByText(/To finish/)).toHaveTextContent("To finish · 2");
  });

  it("keeps To finish when the history read fails, and hides History", async () => {
    invokeMock.mockImplementation(world(new Error("boom"), [draft("d1", "draft")]));
    render(<ActivityView />, { wrapper: wrapper() });
    expect(await screen.findByText(/Failed to load activity: boom/)).toBeInTheDocument();
    expect(await screen.findByTestId("activity-to-finish")).toBeInTheDocument();
    expect(screen.queryByTestId("activity-history")).not.toBeInTheDocument();
  });

  it("filters apply to all lists, with a note, and Clear resets them in one go", async () => {
    invokeMock.mockImplementation(
      world(
        [row({ txid: "aa" }), row({ txid: "bb", action: "bid", name: "foo" })],
        [draft("d1", "draft")],
      ),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    await screen.findByTestId("activity-to-finish");
    fireEvent.change(screen.getByTestId("activity-search-input"), { target: { value: "foo" } });
    await waitFor(() => expect(screen.queryByTestId("activity-to-finish")).not.toBeInTheDocument());
    expect(screen.getAllByTestId("activity-history-row")).toHaveLength(1);

    fireEvent.change(screen.getByTestId("activity-search-input"), { target: { value: "" } });
    const [actionSelect] = screen.getAllByRole("combobox");
    fireEvent.change(actionSelect!, { target: { value: "other" } });
    // The action filter matches the raw action: no row is "other" here.
    await waitFor(() =>
      expect(screen.getByText("No transactions match the current filters.")).toBeInTheDocument(),
    );
    fireEvent.click(screen.getByTestId("activity-clear-filters"));
    await waitFor(() => expect(screen.getByTestId("activity-to-finish")).toBeInTheDocument());
    expect(screen.getAllByTestId("activity-history-row")).toHaveLength(2);
    expect(screen.queryByTestId("activity-clear-filters")).not.toBeInTheDocument();
  });

  it("shows the filter note when a filtered list still has unfinished rows", async () => {
    invokeMock.mockImplementation(world([row({ txid: "aa" })], [draft("d1", "draft")]));
    render(<ActivityView />, { wrapper: wrapper() });
    await screen.findByTestId("activity-to-finish");
    const [, statusSelect] = screen.getAllByRole("combobox");
    fireEvent.change(statusSelect!, { target: { value: "pending" } });
    expect(
      await screen.findByText("Filters apply to To finish and Waiting for a block too."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("No finished transactions match the current filters."),
    ).toBeInTheDocument();
  });

  it("a row click expands fee, block and txid; Copy copies the txid", async () => {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    invokeMock.mockImplementation(world([row({ txid: "abcdef0123456789", height: 437 })], []));
    render(<ActivityView />, { wrapper: wrapper() });
    const tr = await screen.findByTestId("activity-history-row");
    fireEvent.click(tr);
    const detail = screen.getByTestId("activity-row-detail");
    expect(within(detail).getByTestId("activity-block-info-link")).toHaveTextContent("#437");
    fireEvent.click(within(detail).getByTestId("activity-copy-txid"));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("abcdef0123456789"));
    expect(await within(detail).findByText("Copied")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("activity-row-toggle"));
    expect(screen.queryByTestId("activity-row-detail")).not.toBeInTheDocument();
  });

  it("History passes the canonical table contract", async () => {
    invokeMock.mockImplementation(world([row({ txid: "aa" })], []));
    render(<ActivityView />, { wrapper: wrapper() });
    await screen.findByTestId("activity-history-row");
    const { assertCanonicalTable } = await import("../../test/canonicalTable");
    assertCanonicalTable(document.querySelector("table")!, { name: "History" });
  });

  it("ignores drafts of another wallet", async () => {
    invokeMock.mockImplementation(
      world([], [draft("d1", "draft"), draft("d2", "draft", { walletProfileId: "other" })]),
    );
    render(<ActivityView />, { wrapper: wrapper() });
    const finish = await screen.findByTestId("activity-to-finish");
    expect(within(finish).getAllByTestId("activity-to-finish-row")).toHaveLength(1);
  });
});
