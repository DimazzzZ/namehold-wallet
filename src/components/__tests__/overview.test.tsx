import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/plugin-fs", () => ({
  readTextFile: vi.fn(),
  writeTextFile: vi.fn(),
}));

import { Overview } from "../Overview";

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  // get_dashboard_stats returns aggregate, other calls default to []
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === "get_dashboard_stats") {
      return Promise.resolve({
        total: 0,
        staked: 0,
        unstaked: 0,
        by_status: {},
        finalized: 0,
      });
    }
    return Promise.resolve([]);
  });
});

describe("Overview", () => {
  it("renders the overview heading", () => {
    render(<Overview />, { wrapper: wrapper() });
    expect(screen.getAllByText(/overview/i).length).toBeGreaterThan(0);
  });

  it("renders metric labels with mocked aggregate data", () => {
    render(<Overview />, { wrapper: wrapper() });
    // metrics / summary content should appear (total TLDs etc.)
    expect(
      screen.getAllByText(/tld|total|wallet|migration|status/i).length,
    ).toBeGreaterThan(0);
  });
});
