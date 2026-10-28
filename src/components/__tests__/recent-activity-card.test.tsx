import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { RecentActivityCard } from "../activity/RecentActivityCard";
import { SpeedUpButton } from "../SpeedUp";
import { useToFinishCount } from "../../queries/activity";

const profile = { id: "p1", label: "Primary", network: "mainnet", active: true, watchOnly: false };

const chain = (txid: string, over: Record<string, unknown> = {}) => ({
  txid,
  action: "receive",
  name: null,
  nameHash: null,
  valueDoos: 1_000_000,
  direction: "receive",
  height: 100,
  time: 1_700_000_000,
  confirmed: true,
  counterparty: null,
  ...over,
});

const draft = (id: string, status: string, over: Record<string, unknown> = {}) => ({
  id,
  walletProfileId: "p1",
  action: "renew",
  status,
  summary: null,
  errorMessage: null,
  txid: null,
  confirmationHeight: null,
  purchaseLostReason: null,
  createdAt: "2026-08-14 12:00:00",
  ...over,
});

function routes(history: unknown, drafts: unknown[]) {
  return (cmd: string) => {
    switch (cmd) {
      case "list_wallet_profiles":
        return Promise.resolve([profile]);
      case "list_tx_drafts":
        return Promise.resolve(drafts);
      case "read_action_history":
        return history instanceof Error ? Promise.reject(history) : Promise.resolve(history);
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
        <MemoryRouter initialEntries={["/"]}>
          <Routes>
            <Route path="/" element={children} />
            <Route path="/activity" element={<div data-testid="activity-page" />} />
          </Routes>
        </MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => invokeMock.mockReset());

describe("RecentActivityCard", () => {
  it("shows the 5 newest rows in a canonical 4-column table", async () => {
    invokeMock.mockImplementation(
      routes(
        Array.from({ length: 7 }, (_, i) => chain(`t${i}`, { time: 1_700_000_000 - i })),
        [],
      ),
    );
    render(<RecentActivityCard onNameClick={vi.fn()} />, { wrapper: wrapper() });
    await waitFor(() => expect(screen.getAllByTestId("recent-activity-row")).toHaveLength(5));
    const table = document.querySelector("table")!;
    expect(table.querySelectorAll("th")).toHaveLength(4);
    const { assertCanonicalTable } = await import("../../test/canonicalTable");
    assertCanonicalTable(table, { name: "Recent activity" });
  });

  it("a finished row opens its transaction; an unfinished one goes to Activity", async () => {
    invokeMock.mockImplementation(
      routes([chain("abcdef0123456789")], [draft("d1", "draft", { createdAt: "2030-01-01" })]),
    );
    render(<RecentActivityCard onNameClick={vi.fn()} />, { wrapper: wrapper() });
    await waitFor(() => expect(screen.getAllByTestId("recent-activity-row")).toHaveLength(2));
    fireEvent.click(screen.getAllByTestId("recent-activity-row")[1]!); // the receive
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.click(screen.getAllByTestId("recent-activity-row")[0]!); // the draft
    expect(await screen.findByTestId("activity-page")).toBeInTheDocument();
  });

  it("says so when there is nothing yet", async () => {
    invokeMock.mockImplementation(routes([], []));
    render(<RecentActivityCard onNameClick={vi.fn()} />, { wrapper: wrapper() });
    expect(await screen.findByText("No activity yet.")).toBeInTheDocument();
  });
});

describe("useToFinishCount", () => {
  it("counts unfinished drafts even when the history read fails", async () => {
    invokeMock.mockImplementation(
      routes(new Error("no index"), [
        draft("a", "draft"),
        draft("b", "dropped"),
        draft("c", "broadcasted", { txid: "x" }),
        draft("d", "draft", { walletProfileId: "other" }),
      ]),
    );
    const { result } = renderHook(() => useToFinishCount(), { wrapper: wrapper() });
    await waitFor(() => expect(result.current).toBe(2));
  });
});

describe("SpeedUpButton with disabledReason", () => {
  it("is disabled with that reason and never asks the node", async () => {
    invokeMock.mockImplementation(routes([], []));
    render(<SpeedUpButton draftId="d1" disabledReason="A speed-up can't be sped up again" />, {
      wrapper: wrapper(),
    });
    expect(screen.getByTestId("speed-up-button")).toBeDisabled();
    expect(screen.getByTestId("speed-up-reason")).toHaveTextContent(
      "A speed-up can't be sped up again",
    );
    expect(invokeMock.mock.calls.some((c) => c[0] === "speed_up_check")).toBe(false);
  });
});
