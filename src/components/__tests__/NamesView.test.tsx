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
