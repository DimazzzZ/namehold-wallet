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

import { MigrationWorkspace } from "../MigrationWorkspace";

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/migration"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
});

describe("MigrationWorkspace", () => {
  it("renders the Migration page title", () => {
    render(<MigrationWorkspace />, { wrapper: wrapper() });
    expect(screen.getByText(/Migration/i)).toBeInTheDocument();
  });

  it("renders the NamebaseDashboard as the only content (no tabs)", () => {
    render(<MigrationWorkspace />, { wrapper: wrapper() });
    // The page should not show any tab role elements.
    expect(screen.queryByRole("tab")).toBeNull();
    // The NamebaseDashboard heading should be rendered (it's an h2 "Namebase").
    expect(screen.getByRole("heading", { name: /^Namebase$/ })).toBeInTheDocument();
  });
});
