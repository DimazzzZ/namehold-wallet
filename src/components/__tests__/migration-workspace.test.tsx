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
  it("renders the Namebase and Sync tabs", () => {
    render(<MigrationWorkspace />, { wrapper: wrapper() });
    const tabs = screen.getAllByRole("tab");
    const labels = tabs.map((t) => t.textContent ?? "");
    expect(labels.some((l) => /namebase/i.test(l))).toBe(true);
    expect(labels.some((l) => /sync/i.test(l))).toBe(true);
  });

  it("defaults to the Namebase tab as selected", () => {
    render(<MigrationWorkspace />, { wrapper: wrapper() });
    const namebaseTab = screen
      .getAllByRole("tab")
      .find((t) => /namebase/i.test(t.textContent ?? ""))!;
    expect(namebaseTab.getAttribute("aria-selected")).toBe("true");
  });

  it("switches selection to Sync & Verify when clicked", () => {
    render(<MigrationWorkspace />, { wrapper: wrapper() });
    const syncTab = screen
      .getAllByRole("tab")
      .find((t) => /sync/i.test(t.textContent ?? ""))!;
    fireEvent.click(syncTab);
    expect(syncTab.getAttribute("aria-selected")).toBe("true");
  });
});
