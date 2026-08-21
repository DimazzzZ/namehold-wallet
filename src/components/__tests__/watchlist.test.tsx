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

import { Watchlist } from "../Watchlist";

function wrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter initialEntries={["/watchlist"]}>{children}</MemoryRouter>
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

describe("Watchlist", () => {
  it("renders empty state when no watched names", async () => {
    invokeMock.mockImplementation(route({ list_watchlist: [] }));
    render(<Watchlist />, { wrapper: wrapper() });

    expect(
      await screen.findByText(/No names on your watchlist yet/),
    ).toBeInTheDocument();
  });

  it("renders table with watched names", async () => {
    invokeMock.mockImplementation(route());
    render(<Watchlist />, { wrapper: wrapper() });

    // Wait for names to appear
    expect(await screen.findByText(/\.example/)).toBeInTheDocument();
    expect(screen.getByText(/\.testname/)).toBeInTheDocument();
    // Watching count
    expect(screen.getByText(/Watching 2 names/)).toBeInTheDocument();
  });

  it("shows 'Owned' badge for names in the owned set", async () => {
    invokeMock.mockImplementation(route());
    render(<Watchlist />, { wrapper: wrapper() });

    // "example" is in the owned set (read_names returns it)
    expect(await screen.findByText("Owned")).toBeInTheDocument();
  });

  it("add button adds a name", async () => {
    invokeMock.mockImplementation(route({ add_to_watchlist: null }));
    render(<Watchlist />, { wrapper: wrapper() });

    const input = await screen.findByTestId("watchlist-add-input");
    fireEvent.change(input, { target: { value: "newname" } });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("add_to_watchlist", { name: "newname" });
    });
  });

  it("remove button removes a name", async () => {
    invokeMock.mockImplementation(route({ remove_from_watchlist: null }));
    render(<Watchlist />, { wrapper: wrapper() });

    // Wait for table to render
    await screen.findByText(/\.example/);
    const removeButtons = screen.getAllByRole("button", { name: "Remove" });
    fireEvent.click(removeButtons[0]!);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("remove_from_watchlist", { name: "example" });
    });
  });

  it("tag cell editable: click opens input, blur saves", async () => {
    invokeMock.mockImplementation(route({ update_watchlist_tags: null }));
    render(<Watchlist />, { wrapper: wrapper() });

    // Wait for tags to render
    await screen.findByText("auction");
    // Click the tag cell for "example" row (has "auction, hot" tags).
    // There's one tag button per row; the first row is "example".
    const tagButtons = screen.getAllByTitle("Click to edit tags");
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

  it("name cell click opens NameInfoModal", async () => {
    invokeMock.mockImplementation(route());
    render(<Watchlist />, { wrapper: wrapper() });

    const nameButton = await screen.findByText(/\.example/);
    fireEvent.click(nameButton);

    // NameInfoModal renders — it typically shows the name prominently
    await waitFor(() => {
      // The modal will call invoke to fetch name info; just confirm it rendered
      expect(invokeMock).toHaveBeenCalledWith("read_name_info", { name: "example" });
    });
  });

  it("export CSV button calls invoke", async () => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    vi.mocked(save).mockResolvedValue("/tmp/export.csv");
    invokeMock.mockImplementation(route({ export_watchlist_csv: 2 }));
    render(<Watchlist />, { wrapper: wrapper() });

    await screen.findByText(/\.example/);
    const exportBtn = screen.getByRole("button", { name: /Export CSV/ });
    fireEvent.click(exportBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("export_watchlist_csv", { path: "/tmp/export.csv" });
    });
  });

  it("export CSV button disabled when watchlist is empty", async () => {
    invokeMock.mockImplementation(route({ list_watchlist: [] }));
    render(<Watchlist />, { wrapper: wrapper() });

    await screen.findByText(/No names on your watchlist yet/);
    const exportBtn = screen.getByRole("button", { name: /Export CSV/ });
    expect(exportBtn).toBeDisabled();
  });
});
