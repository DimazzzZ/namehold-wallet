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
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn(),
  readText: vi.fn().mockResolvedValue(""),
}));

import { Settings } from "../Settings";
import { NodeControl } from "../NodeControl";

function wrapper(path: string) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <MemoryRouter initialEntries={[path]}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue([]);
});

describe("Settings vs Node responsibility separation", () => {
  it("Settings renders configuration/safety content", () => {
    render(<Settings />, { wrapper: wrapper("/settings") });
    expect(
      screen.getAllByText(/settings|connection|write|api|safety/i).length,
    ).toBeGreaterThan(0);
  });

  it("NodeControl renders runtime/lifecycle content", () => {
    render(<NodeControl />, { wrapper: wrapper("/node") });
    expect(screen.getAllByText(/node|status|start|stop|sync/i).length).toBeGreaterThan(
      0,
    );
  });

  it("both pages render independently without crashing", () => {
    const { container: settingsContainer } = render(<Settings />, {
      wrapper: wrapper("/settings"),
    });
    const { container: nodeContainer } = render(<NodeControl />, {
      wrapper: wrapper("/node"),
    });
    expect(settingsContainer).toBeTruthy();
    expect(nodeContainer).toBeTruthy();
  });
});
