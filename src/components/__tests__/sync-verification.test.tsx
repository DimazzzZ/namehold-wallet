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

import { SyncVerification } from "../SyncVerification";

function wrapper() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  };
}

function routeInvoke(report: unknown) {
  return (cmd: string) => {
    switch (cmd) {
      case "compare_inventory_with_provider":
        return Promise.resolve(report);
      case "get_audit_log":
        return Promise.resolve([]);
      default:
        return Promise.resolve(null);
    }
  };
}

beforeEach(() => invokeMock.mockReset());

describe("SyncVerification — compare inventory vs Namebase", () => {
  it("shows the three-way breakdown with counts after comparing", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        providerKind: "namebase",
        providerLabel: "Namebase",
        matched: ["examplename", "exampletld"],
        missingAtProvider: ["gone"],
        extraAtProvider: ["extra1", "extra2"],
      }),
    );
    render(<SyncVerification />, { wrapper: wrapper() });

    fireEvent.click(screen.getByRole("button", { name: /Compare inventory/i }));

    // Summary counts (always visible) reflect each bucket.
    expect(await screen.findByText(/Still at Namebase: 2/i)).toBeInTheDocument();
    expect(screen.getByText(/Left Namebase \/ elsewhere: 1/i)).toBeInTheDocument();
    expect(screen.getByText(/On Namebase only: 2/i)).toBeInTheDocument();
    // The relabeled bucket sections render their names.
    expect(screen.getByText(/\.gone/)).toBeInTheDocument();
    expect(screen.getByText(/\.exampletld/)).toBeInTheDocument();
  });

  it("still shows a completed summary when every bucket is empty (not blank)", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        providerKind: "namebase",
        providerLabel: "Namebase",
        matched: [],
        missingAtProvider: [],
        extraAtProvider: [],
      }),
    );
    render(<SyncVerification />, { wrapper: wrapper() });

    fireEvent.click(screen.getByRole("button", { name: /Compare inventory/i }));

    // A result panel appears even with empty buckets (the old bug showed nothing).
    expect(await screen.findByTestId("compare-report")).toBeInTheDocument();
    expect(screen.getByText(/Still at Namebase: 0/i)).toBeInTheDocument();
    expect(screen.getByText(/import your domains on the Namebase tab first/i)).toBeInTheDocument();
  });

  it("keeps the last comparison after navigating away and back (no recompute)", async () => {
    invokeMock.mockImplementation(
      routeInvoke({
        providerKind: "namebase",
        providerLabel: "Namebase",
        matched: ["examplename"],
        missingAtProvider: [],
        extraAtProvider: [],
      }),
    );
    // A single shared QueryClient mimics the app — the cache survives unmounts.
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const Wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>
        <MemoryRouter>{children}</MemoryRouter>
      </QueryClientProvider>
    );

    const view = render(<SyncVerification />, { wrapper: Wrapper });
    fireEvent.click(screen.getByRole("button", { name: /Compare inventory/i }));
    expect(await screen.findByText(/Still at Namebase: 1/i)).toBeInTheDocument();
    const comparesAfterFirst = invokeMock.mock.calls.filter(
      (c) => c[0] === "compare_inventory_with_provider",
    ).length;

    // Navigate away (unmount) and back (remount) against the same client.
    view.unmount();
    render(<SyncVerification />, { wrapper: Wrapper });

    // The previous result is shown immediately, without re-running compare.
    expect(await screen.findByText(/Still at Namebase: 1/i)).toBeInTheDocument();
    await waitFor(() => {
      const comparesNow = invokeMock.mock.calls.filter(
        (c) => c[0] === "compare_inventory_with_provider",
      ).length;
      expect(comparesNow).toBe(comparesAfterFirst);
    });
  });
});
