import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, useLocation } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

import { NamesView } from "../NamesView";
import { makeCapabilities } from "../../test/fixtures/capabilities";

const profile = (over: Record<string, unknown> = {}) => ({
  id: "p1",
  label: "Primary",
  kind: "mnemonic_hot",
  network: "mainnet",
  watchOnly: false,
  hasPassphrase: true,
  active: true,
  ...over,
});

const owned = (name: string, state = "CLOSED") => ({
  name,
  state,
  height: 10,
  renewal: 20,
  owner: { hash: "t", index: 0 },
  stats: null,
});

function route(p = profile()) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([p]);
      case "read_names":
        return Promise.resolve([owned("alpha", "REVEAL"), owned("bravo")]);
      case "get_names_action_capabilities":
        return Promise.resolve([
          makeCapabilities({ name: "alpha", taskState: "readyToReveal", phase: "REVEAL" }),
          makeCapabilities({ name: "bravo", taskState: "ownedNoUrgentAction" }),
        ]);
      case "list_watchlist":
      case "list_tx_drafts":
      case "read_auction_position_names":
        return Promise.resolve([]);
      default:
        return Promise.resolve(null);
    }
  };
}

function Where() {
  const l = useLocation();
  return <div data-testid="where">{l.search}</div>;
}

function wrap(initial = "/names") {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={[initial]}>
          {children}
          <Where />
        </MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("NamesView", () => {
  it("binds the chips to ?filter; an unknown value means All", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrap("/names?filter=nonsense") });
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(2));
    expect(screen.getByTestId("names-chip-all")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByTestId("names-chip-owned"));
    expect(screen.getByTestId("where")).toHaveTextContent("?filter=owned");
    expect(screen.getAllByTestId("names-row")).toHaveLength(1);
    fireEvent.click(screen.getByTestId("names-chip-all"));
    expect(screen.getByTestId("where")).toBeEmptyDOMElement();
  });

  it("shows each chip's empty text", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrap("/names?filter=watching") });
    expect(await screen.findByTestId("names-empty")).toHaveTextContent(
      "No names on your watchlist yet. Look up a name above and choose Watch.",
    );
  });

  it("watch-only: no checkboxes, Batch bid disabled, signing actions become View", async () => {
    invokeMock.mockImplementation(route(profile({ watchOnly: true, kind: "watch_only_xpub" })));
    render(<NamesView />, { wrapper: wrap() });
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(2));
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(screen.getByTestId("open-batch-bid-btn")).toBeDisabled();
    // Capabilities are not fetched for watch-only, so nothing asks to sign.
    expect(invokeMock.mock.calls.some((c) => c[0] === "get_names_action_capabilities")).toBe(false);
    expect(screen.queryByRole("button", { name: "Reveal" })).toBeNull();
  });

  it("Esc dismisses the lookup result card", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrap() });
    const input = await screen.findByTestId("names-lookup-input");
    fireEvent.change(input, { target: { value: "newname" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(await screen.findByTestId("names-lookup-result")).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("names-lookup-result")).toBeNull());
  });

  it("the table passes the canonical contract", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrap() });
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(2));
    const { assertCanonicalTable } = await import("../../test/canonicalTable");
    assertCanonicalTable(document.querySelector("table")!, { name: "Names" });
  });
});

describe("NamesView — sort and filters", () => {
  const owned2 = (name: string, days: number, blocks: number, value: number) => ({
    ...owned(name),
    value,
    stats: { daysUntilExpire: days, blocksUntilExpire: blocks },
  });
  function world() {
    return (cmd: string) => {
      switch (cmd) {
        case "list_wallet_profiles":
          return Promise.resolve([profile()]);
        case "read_names":
          return Promise.resolve([
            owned2("charlie", 300, 43200, 1_000_000),
            owned2("ab", 10, 1440, 9_000_000),
            owned2("bravo", 60, 8640, 5_000_000),
          ]);
        case "get_names_action_capabilities":
          return Promise.resolve(
            ["charlie", "ab", "bravo"].map((n) =>
              makeCapabilities({ name: n, taskState: "ownedNoUrgentAction" }),
            ),
          );
        case "list_watchlist":
          return Promise.resolve([{ name: "ab", addedAt: "", notes: "", tags: "design, short" }]);
        case "list_tx_drafts":
        case "read_auction_position_names":
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    };
  }
  const order = () =>
    screen.getAllByTestId("names-row").map((r) => r.querySelector(".font-mono")?.textContent);

  it("sorts by a header click and reverses on the second", async () => {
    invokeMock.mockImplementation(world());
    render(<NamesView />, { wrapper: wrap() });
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(3));
    fireEvent.click(screen.getByTestId("names-sort-expires"));
    expect(order()).toEqual([".ab", ".bravo", ".charlie"]);
    expect(screen.getByTestId("where")).toHaveTextContent("?sort=expires");
    expect(screen.getByTestId("names-sort-expires").closest("th")).toHaveAttribute(
      "aria-sort",
      "ascending",
    );
    fireEvent.click(screen.getByTestId("names-sort-expires"));
    expect(order()).toEqual([".charlie", ".bravo", ".ab"]);
    fireEvent.click(screen.getByTestId("names-sort-value"));
    expect(order()).toEqual([".ab", ".bravo", ".charlie"]); // high → low
  });

  it("filters from the popover, shows pills, and Clear filters resets them", async () => {
    invokeMock.mockImplementation(world());
    render(<NamesView />, { wrapper: wrap() });
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(3));
    fireEvent.click(screen.getByTestId("names-filters-button"));
    fireEvent.click(screen.getByTestId("names-filter-expires-90"));
    expect(order()).toEqual([".ab", ".bravo"]);
    fireEvent.click(screen.getByTestId("names-filter-tag-design"));
    expect(order()).toEqual([".ab"]);
    expect(screen.getByTestId("names-filters-button")).toHaveTextContent("Filters2");
    expect(screen.getByTestId("names-active-filters")).toHaveTextContent("Expires within 90 days");
    expect(screen.getByTestId("names-count")).toHaveTextContent("1 of 3");
    fireEvent.click(screen.getAllByTestId("names-active-filter-remove")[1]!); // the tag
    expect(order()).toEqual([".ab", ".bravo"]);
    fireEvent.click(screen.getByTestId("names-filter-len-1-3"));
    expect(order()).toEqual([".ab"]);
    fireEvent.click(screen.getByText("Clear filters"));
    expect(screen.getAllByTestId("names-row")).toHaveLength(3);
    expect(screen.queryByTestId("names-active-filters")).toBeNull();
  });

  it("text filter matches tags; an empty result offers Clear filters", async () => {
    invokeMock.mockImplementation(world());
    render(<NamesView />, { wrapper: wrap("/names?chars=hyphen") });
    expect(await screen.findByText("No names match these filters.")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("names-filters-empty-clear"));
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(3));
    fireEvent.change(screen.getByTestId("names-filter"), { target: { value: "desig" } });
    expect(order()).toEqual([".ab"]);
  });

  it("a chip keeps the sort and filters in the URL", async () => {
    invokeMock.mockImplementation(world());
    render(<NamesView />, { wrapper: wrap("/names?sort=value&len=4-5") });
    await waitFor(() => expect(screen.getAllByTestId("names-row").length).toBeGreaterThan(0));
    fireEvent.click(screen.getByTestId("names-chip-owned"));
    const search = screen.getByTestId("where").textContent!;
    expect(search).toContain("filter=owned");
    expect(search).toContain("sort=value");
    expect(search).toContain("len=4-5");
  });

  it("regtest: Expires options and filter use blocks", async () => {
    const base = world();
    invokeMock.mockImplementation((cmd: string) =>
      cmd === "list_wallet_profiles"
        ? Promise.resolve([profile({ network: "regtest" })])
        : base(cmd),
    );
    render(<NamesView />, { wrapper: wrap() });
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(3));
    fireEvent.click(screen.getByTestId("names-filters-button"));
    expect(screen.getByTestId("names-filter-expires-30")).toHaveTextContent("Within 100 blocks");
    fireEvent.click(screen.getByTestId("names-filter-expires-365"));
    // ≤ 10,000 blocks: ab (1,440) and bravo (8,640), not charlie (43,200)
    expect(order()).toEqual([".ab", ".bravo"]);
  });

  it("drops selected rows the filters hide", async () => {
    invokeMock.mockImplementation(world());
    render(<NamesView />, { wrapper: wrap() });
    await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(3));
    fireEvent.click(screen.getByLabelText("Select all names"));
    expect(screen.getByTestId("batch-action-bar")).toHaveTextContent("3 selected");
    fireEvent.click(screen.getByTestId("names-filters-button"));
    fireEvent.click(screen.getByTestId("names-filter-expires-90"));
    await waitFor(() =>
      expect(screen.getByTestId("batch-action-bar")).toHaveTextContent("2 selected"),
    );
  });

  describe("ICANN collision (spec R4, R7)", () => {
    const icannWorld =
      (pendingAgent = false) =>
      (cmd: string) => {
        switch (cmd) {
          case "list_wallet_profiles":
            return Promise.resolve([profile()]);
          case "read_names":
            return Promise.resolve([owned("agent"), owned("zzzz-not-applied")]);
          case "get_names_action_capabilities":
            return Promise.resolve([
              makeCapabilities({
                name: "agent",
                taskState: "ownedNoUrgentAction",
                pendingBroadcastAction: pendingAgent ? "RENEW" : null,
              }),
              makeCapabilities({ name: "zzzz-not-applied", taskState: "ownedNoUrgentAction" }),
            ]);
          case "list_watchlist":
          case "list_tx_drafts":
          case "read_auction_position_names":
            return Promise.resolve([]);
          default:
            return Promise.resolve(null);
        }
      };
    const rowOf = (name: string) =>
      screen
        .getAllByTestId("names-row")
        .find((r) => r.querySelector(".font-mono")?.textContent === `.${name}`)!;

    it("a colliding row shows the ICANN badge with the applicant count; a plain row does not", async () => {
      invokeMock.mockImplementation(icannWorld());
      render(<NamesView />, { wrapper: wrap() });
      await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(2));
      const badge = rowOf("agent").querySelector('[data-testid="icann-badge"]');
      expect(badge).toHaveTextContent("ICANN");
      fireEvent.mouseEnter(badge!.parentElement!); // the Tooltip's reference span
      await waitFor(() =>
        expect(screen.getByRole("tooltip")).toHaveTextContent(
          "Applied for as a gTLD in ICANN's 2026 round by 11 applicants",
        ),
      );
      expect(rowOf("zzzz-not-applied").querySelector('[data-testid="icann-badge"]')).toBeNull();
    });

    it("the pending-broadcast badge wins over the ICANN badge (one badge per row)", async () => {
      invokeMock.mockImplementation(icannWorld(true));
      render(<NamesView />, { wrapper: wrap() });
      await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(2));
      // Capabilities arrive after the rows: wait for the pending badge they carry.
      await screen.findByTestId("auction-pending-block");
      const agent = rowOf("agent");
      expect(agent.querySelector('[data-testid="auction-pending-block"]')).not.toBeNull();
      expect(agent.querySelector('[data-testid="icann-badge"]')).toBeNull();
    });

    it("the ICANN filter keeps colliding rows, shows a pill and lives in the URL", async () => {
      invokeMock.mockImplementation(icannWorld());
      render(<NamesView />, { wrapper: wrap() });
      await waitFor(() => expect(screen.getAllByTestId("names-row")).toHaveLength(2));
      fireEvent.click(screen.getByTestId("names-filters-button"));
      fireEvent.click(screen.getByTestId("names-filter-icann-collision"));
      expect(order()).toEqual([".agent"]);
      expect(screen.getByTestId("names-active-filters")).toHaveTextContent("ICANN collision");
      expect(screen.getByTestId("where")).toHaveTextContent("?icann=collision");
      fireEvent.click(screen.getByTestId("names-active-filter-remove"));
      expect(order()).toEqual([".agent", ".zzzz-not-applied"]);
      expect(screen.getByTestId("where")).not.toHaveTextContent("icann");
    });
  });
});
