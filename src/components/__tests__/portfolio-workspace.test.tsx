import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
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

import { PortfolioWorkspace } from "../PortfolioWorkspace";

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={["/portfolio"]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
});

describe("PortfolioWorkspace", () => {
  it("renders the four section tabs", () => {
    render(<PortfolioWorkspace />, { wrapper: wrapper() });
    const tabs = screen.getAllByRole("tab");
    const labels = tabs.map((t) => t.textContent ?? "");
    expect(labels.some((l) => /inventory/i.test(l))).toBe(true);
    expect(labels.some((l) => /batches/i.test(l))).toBe(true);
    expect(labels.some((l) => /renewals/i.test(l))).toBe(true);
    expect(labels.some((l) => /dns/i.test(l))).toBe(true);
  });

  it("defaults to the Inventory tab as selected", () => {
    render(<PortfolioWorkspace />, { wrapper: wrapper() });
    const inventoryTab = screen
      .getAllByRole("tab")
      .find((t) => /inventory/i.test(t.textContent ?? ""))!;
    expect(inventoryTab.getAttribute("aria-selected")).toBe("true");
  });

  it("switches selection when a different tab is clicked", () => {
    render(<PortfolioWorkspace />, { wrapper: wrapper() });
    const batchesTab = screen
      .getAllByRole("tab")
      .find((t) => /batches/i.test(t.textContent ?? ""))!;
    fireEvent.click(batchesTab);
    expect(batchesTab.getAttribute("aria-selected")).toBe("true");
  });
});
