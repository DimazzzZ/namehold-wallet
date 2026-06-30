import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter } from "react-router-dom";
import { Settings } from "../Settings";
import { NodeControl } from "../NodeControl";
import { WalletManager } from "../WalletManager";
import { SyncVerification } from "../SyncVerification";
import { Renewals } from "../Renewals";
import { DnsRecords } from "../DnsRecords";
import { Batches } from "../Batches";
import { WalletView } from "../WalletView";
import { TldInventory } from "../TldInventory";
import { Layout } from "../Layout";
import { Onboarding } from "../Onboarding";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue(null), save: vi.fn().mockResolvedValue(null) }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: vi.fn().mockResolvedValue(undefined), readText: vi.fn().mockResolvedValue("") }));

function renderWithProviders(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={qc}><BrowserRouter>{ui}</BrowserRouter></QueryClientProvider>);
}

describe("Page Components - Smoke Tests", () => {
  it("Settings renders", () => {
    const { container } = renderWithProviders(<Settings />);
    expect(container).toBeTruthy();
  });

  it("NodeControl renders", () => {
    const { container } = renderWithProviders(<NodeControl />);
    expect(container).toBeTruthy();
  });

  it("WalletManager renders", () => {
    const { container } = renderWithProviders(<WalletManager />);
    expect(container).toBeTruthy();
  });

  it("SyncVerification renders", () => {
    const { container } = renderWithProviders(<SyncVerification />);
    expect(container).toBeTruthy();
  });

  it("Renewals renders", () => {
    const { container } = renderWithProviders(<Renewals />);
    expect(container).toBeTruthy();
  });

  it("DnsRecords renders", () => {
    const { container } = renderWithProviders(<DnsRecords />);
    expect(container).toBeTruthy();
  });

  it("Batches renders", () => {
    const { container } = renderWithProviders(<Batches />);
    expect(container).toBeTruthy();
  });

  it("WalletView renders", () => {
    const { container } = renderWithProviders(<WalletView />);
    expect(container).toBeTruthy();
  });

  it("TldInventory renders", () => {
    const { container } = renderWithProviders(<TldInventory />);
    expect(container).toBeTruthy();
  });

  it("Layout renders", () => {
    const { container } = renderWithProviders(<Layout />);
    expect(container).toBeTruthy();
  });

  it("Onboarding renders", () => {
    const { container } = renderWithProviders(<Onboarding />);
    expect(container).toBeTruthy();
  });
});
