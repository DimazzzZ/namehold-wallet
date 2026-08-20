import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { AddWalletForm } from "../AddWalletForm";

function wrap(ui: ReactNode): ReactNode {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return <QueryClientProvider client={qc}>{ui}</QueryClientProvider>;
}

describe("AddWalletForm — Ledger import path", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("shows the Ledger option on the choose screen", () => {
    render(wrap(<AddWalletForm onDone={vi.fn()} />));
    expect(screen.getByText(/Connect a Ledger device/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Keys stay on the device; every spend is confirmed on-device/i),
    ).toBeInTheDocument();
  });

  it("navigates to the Ledger screen and shows device prep instructions", () => {
    render(wrap(<AddWalletForm onDone={vi.fn()} />));

    fireEvent.click(screen.getByText(/Connect a Ledger device/i));

    expect(
      screen.getByText(
        /Make sure your Ledger is connected via USB, unlocked, and the Handshake/i,
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Import from Ledger/i })).toBeInTheDocument();
  });

  it("invokes import_ledger_profile with label + network on submit", async () => {
    invokeMock.mockResolvedValueOnce({ id: "p-ledger" });
    const onDone = vi.fn();

    render(wrap(<AddWalletForm onDone={onDone} defaultLabel="MyLedger" />));
    fireEvent.click(screen.getByText(/Connect a Ledger device/i));
    fireEvent.click(screen.getByRole("button", { name: /Import from Ledger/i }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("import_ledger_profile", {
        label: "MyLedger",
        network: "mainnet",
      });
    });
    await waitFor(() => expect(onDone).toHaveBeenCalled());
  });

  it("shows 'Connecting to device...' while the import is pending", async () => {
    // Use a promise we control so the button label reflects the pending state.
    let resolve: (v: unknown) => void = () => {};
    invokeMock.mockImplementationOnce(
      () => new Promise((r) => (resolve = r)),
    );

    render(wrap(<AddWalletForm onDone={vi.fn()} defaultLabel="L" />));
    fireEvent.click(screen.getByText(/Connect a Ledger device/i));
    fireEvent.click(screen.getByRole("button", { name: /Import from Ledger/i }));

    await waitFor(() => {
      expect(
        screen.getByRole("button", { name: /Connecting to device/i }),
      ).toBeInTheDocument();
    });

    // Resolve to clean up the pending mutation.
    resolve({ id: "p" });
  });
});
