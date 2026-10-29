import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
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

import { NamesView } from "../NamesView";

function wrapper(at = "/names?filter=watching") {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={[at]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

const watchedNames = [
  { name: "example", addedAt: "2025-01-15T10:00:00Z", notes: "", tags: "auction, hot" },
  { name: "testname", addedAt: "2025-02-01T12:00:00Z", notes: "", tags: "" },
];

const statuses = [
  { name: "example", watched: true, tags: "auction, hot", state: "BIDDING", expiry: null },
  { name: "testname", watched: true, tags: "", state: "CLOSED", expiry: 1800000000 },
];

function route(overrides: Record<string, unknown> = {}) {
  return (cmd: string, args?: Record<string, unknown>) => {
    if (cmd in overrides) return Promise.resolve(overrides[cmd]);
    switch (cmd) {
      case "list_watchlist":
        return Promise.resolve(watchedNames);
      case "get_watchlist_status":
        return Promise.resolve(statuses);
      case "list_tx_drafts":
      case "get_names_action_capabilities":
        return Promise.resolve([]);
      case "list_wallet_profiles":
        return Promise.resolve([{ id: "p1", network: "mainnet", active: true, watchOnly: false }]);
      case "read_name_info":
        if (args?.name === "example") {
          return Promise.resolve({
            name: "example",
            state: "BIDDING",
            highest: 5_000_000,
            stats: { daysUntilExpire: null },
          });
        }
        if (args?.name === "testname") {
          return Promise.resolve({
            name: "testname",
            state: "CLOSED",
            highest: 0,
            stats: { daysUntilExpire: 45 },
          });
        }
        return Promise.resolve(null);
      case "read_names":
        // useReadNames() calls this; return one owned name to test "Owned" badge
        return Promise.resolve([{ name: "example", state: "CLOSED" }]);
      default:
        return Promise.resolve(null);
    }
  };
}

beforeEach(() => {
  invokeMock.mockReset();
});

// The watchlist lives on the Names page now (chip Watching).
describe("NamesView — watchlist", () => {
  it("renders empty state when no watched names", async () => {
    invokeMock.mockImplementation(route({ list_watchlist: [] }));
    render(<NamesView />, { wrapper: wrapper() });

    expect(await screen.findByTestId("names-empty")).toHaveTextContent(
      "No names on your watchlist yet. Look up a name above and choose Watch.",
    );
  });

  it("renders table with watched names", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrapper() });

    // Wait for names to appear
    expect(await screen.findByText(/\.example/)).toBeInTheDocument();
    expect(screen.getByText(/\.testname/)).toBeInTheDocument();
    // Watching count, on its chip
    expect(screen.getByTestId("names-chip-watching")).toHaveTextContent("Watching 2");
  });

  it("a watched name we own is one owned row, not a second watch row", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrapper() });

    // "example" is in the owned set (read_names returns it as CLOSED)
    await screen.findByText(/\.example/);
    expect(screen.getAllByTestId("names-row")).toHaveLength(2);
    expect(screen.getByTestId("names-chip-owned")).toHaveTextContent("Owned 1");
    const row = screen.getAllByTestId("names-row")[0]!;
    expect(row).toHaveTextContent(".example");
    // The owned row's phase, not the watchlist's cached BIDDING.
    expect(within(row).getByText("Closed")).toBeInTheDocument();
    expect(within(row).getByText("auction")).toBeInTheDocument();
  });

  it("a watched name still being bought through Shakedex shows as Bought", async () => {
    invokeMock.mockImplementation(
      route({
        // The row `read_shakedex_purchase_names` adds to `read_names`.
        read_names: [
          {
            name: "example",
            state: null,
            height: null,
            renewal: null,
            owner: null,
            owner_address: "hs1qdest",
            registered: true,
            expired: null,
            stats: null,
            shakedex: {
              state: "awaitingFinalize",
              blocksRemaining: 3,
              purchaseId: "p1",
              purchaseDraftId: "draft-p1",
              priceDoos: 5_000_000,
              lockupBlocks: 288,
              wallClockTiming: true,
            },
          },
        ],
      }),
    );
    render(<NamesView />, { wrapper: wrapper() });
    await screen.findByText(/\.example/);
    await waitFor(() =>
      expect(invokeMock.mock.calls.some((c) => c[0] === "read_names")).toBe(true),
    );
    const row = (await screen.findByText(/\.example/)).closest("tr")!;
    await waitFor(() => expect(within(row).getByText("Bought")).toBeInTheDocument());
    expect(within(row).queryByText("Closed")).toBeNull();
  });

  it("Watch on the lookup result adds a name", async () => {
    invokeMock.mockImplementation(route({ add_to_watchlist: null }));
    render(<NamesView />, { wrapper: wrapper() });

    const input = await screen.findByTestId("names-lookup-input");
    fireEvent.change(input, { target: { value: "newname" } });
    fireEvent.keyDown(input, { key: "Enter" });
    fireEvent.click(await screen.findByTestId("names-lookup-watch"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("add_to_watchlist", { name: "newname" });
    });
  });

  it("Remove from watchlist in the row's More menu removes a name", async () => {
    invokeMock.mockImplementation(route({ remove_from_watchlist: null }));
    render(<NamesView />, { wrapper: wrapper() });

    // Wait for table to render
    await screen.findByText(/\.example/);
    fireEvent.click(screen.getAllByRole("button", { name: "More" })[0]!);
    fireEvent.click(await screen.findByRole("menuitem", { name: /Remove from watchlist/ }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("remove_from_watchlist", { name: "example" });
    });
  });

  it("tag cell editable: click opens input, blur saves", async () => {
    invokeMock.mockImplementation(route({ update_watchlist_tags: null }));
    render(<NamesView />, { wrapper: wrapper() });

    // Wait for tags to render
    await screen.findByText("auction");
    // Click the tag cell for "example" row (has "auction, hot" tags).
    // There's one tag button per row; the first row is "example".
    const tagButtons = screen.getAllByTestId("watchlist-tags-cell");
    fireEvent.click(tagButtons[0]!);

    // Input appears with current value
    const tagInput = await screen.findByPlaceholderText("tag1, tag2");
    expect(tagInput).toHaveValue("auction, hot");

    // Change and blur
    fireEvent.change(tagInput, { target: { value: "auction, premium" } });
    fireEvent.blur(tagInput);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("update_watchlist_tags", {
        name: "example",
        tags: "auction, premium",
      });
    });
  });

  it("row click opens the unified name modal", async () => {
    invokeMock.mockImplementation(route());
    render(<NamesView />, { wrapper: wrapper() });

    const name = await screen.findByText(/\.example/);
    fireEvent.click(name.closest("tr")!);

    // NameActionsModal renders — it typically shows the name prominently
    await waitFor(() => {
      // The modal will call invoke to fetch name info; just confirm it rendered
      expect(invokeMock).toHaveBeenCalledWith("read_name_info", { name: "example" });
    });
  });

  it("export CSV button calls invoke", async () => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(save).mockResolvedValue("/tmp/export.csv");
    invokeMock.mockImplementation(route({ export_watchlist_csv: 2 }));
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByText(/\.example/);
    const exportBtn = screen.getByRole("button", { name: /Export CSV/ });
    fireEvent.click(exportBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("export_watchlist_csv", { path: "/tmp/export.csv" });
    });
  });

  it("export CSV button disabled when watchlist is empty", async () => {
    invokeMock.mockImplementation(route({ list_watchlist: [] }));
    render(<NamesView />, { wrapper: wrapper() });

    await screen.findByTestId("names-empty");
    const exportBtn = screen.getByRole("button", { name: /Export CSV/ });
    expect(exportBtn).toBeDisabled();
  });
});
