import { describe, it, expect, vi } from "vitest";
import "@testing-library/jest-dom";
import { act, render, screen } from "@testing-library/react";
import { PageHeader } from "../PageHeader";

describe("PageHeader", () => {
  it("shows an action's tooltip on focus", async () => {
    render(
      <PageHeader
        title="Wallet"
        actions={[{ label: "Sync", onClick: vi.fn(), tooltip: "Scan the chain" }]}
      />,
    );
    const button = screen.getByRole("button", { name: "Sync" });
    act(() => button.focus());
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Scan the chain");
  });

  it("keeps the label (not a placeholder) while an action is loading", () => {
    render(<PageHeader title="Wallet" actions={[{ label: "Syncing…", loading: true }]} />);
    expect(screen.getByRole("button", { name: /Syncing…/ })).toBeDisabled();
  });
});
