import { describe, it, expect, vi } from "vitest";
import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter } from "react-router-dom";
import { Settings } from "../Settings";
import { WalletView } from "../WalletView";
import { Layout } from "../Layout";
import { Onboarding } from "../Onboarding";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue({}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
  save: vi.fn().mockResolvedValue(null),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: vi.fn().mockResolvedValue(undefined),
  readText: vi.fn().mockResolvedValue(""),
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
  enable: vi.fn().mockResolvedValue(undefined),
  disable: vi.fn().mockResolvedValue(undefined),
  isEnabled: vi.fn().mockResolvedValue(false),
}));

function renderWithProviders(ui: React.ReactElement) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={qc}>
      <BrowserRouter>{ui}</BrowserRouter>
    </QueryClientProvider>,
  );
}

describe("Page Components - Smoke Tests", () => {
  it("Settings renders", () => {
    const { container } = renderWithProviders(<Settings />);
    expect(container).toBeTruthy();
  });

  it("WalletView renders", () => {
    const { container } = renderWithProviders(<WalletView />);
    expect(container).toBeTruthy();
  });

  it("Layout renders", () => {
    const { container } = renderWithProviders(<Layout />);
    expect(container).toBeTruthy();
  });

  it("Layout lists Market right after Auctions, marked New", () => {
    renderWithProviders(<Layout />);
    const labels = Array.from(document.querySelectorAll("nav a")).map((a) =>
      a.textContent?.replace("New", "").trim(),
    );
    expect(labels.indexOf("Market")).toBe(labels.indexOf("Auctions") + 1);
    expect(document.querySelector('[data-testid="nav-badge-market"]')).toHaveTextContent("New");
    expect(document.querySelectorAll('[data-testid^="nav-badge-"]')).toHaveLength(1);
  });

  it("Onboarding renders", () => {
    const { container } = renderWithProviders(<Onboarding />);
    expect(container).toBeTruthy();
  });
});
