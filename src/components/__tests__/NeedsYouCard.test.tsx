import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { NeedsYouCard } from "../NeedsYouCard";
import type { NeedsYouItem } from "../../lib/needsYou";

const item = (over: Partial<NeedsYouItem>): NeedsYouItem => ({
  kind: "reveal",
  name: "cryptohq",
  text: "Reveal your bid",
  when: "5 blocks",
  tone: "warn",
  actionLabel: "Reveal",
  blocks: 5,
  ...over,
});

const ITEMS = [
  item({}),
  item({
    kind: "finalizePurchase",
    name: "pixelart",
    text: "Your purchase is ready",
    when: "Ready now",
    tone: "ok",
    actionLabel: "Finalize",
    purchaseId: "pur-1",
  }),
];

function renderCard(over: Partial<React.ComponentProps<typeof NeedsYouCard>> = {}) {
  const props = {
    items: ITEMS,
    isLoading: false,
    isDegraded: false,
    onOpenName: vi.fn(),
    onFinalizePurchase: vi.fn(),
    finalizingId: null,
    finalizeRefusal: null,
    ...over,
  };
  render(<NeedsYouCard {...props} />);
  return props;
}

describe("NeedsYouCard", () => {
  it("renders the rows in the order given, with the count", () => {
    renderCard();
    const rows = screen.getAllByTestId("needs-you-row");
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringContaining(".cryptohq"),
      expect.stringContaining(".pixelart"),
    ]);
    expect(screen.getByRole("heading")).toHaveTextContent("Needs you · 2");
  });

  it("opens the name from its link and its action button", () => {
    const props = renderCard();
    fireEvent.click(screen.getByRole("button", { name: ".cryptohq" }));
    fireEvent.click(screen.getByRole("button", { name: "Reveal" }));
    expect(props.onOpenName).toHaveBeenCalledTimes(2);
    expect(props.onOpenName).toHaveBeenCalledWith("cryptohq");
  });

  it("shows a purchase's name as plain text and finalizes it in place", () => {
    const props = renderCard();
    const row = screen.getAllByTestId("needs-you-row")[1]!;
    expect(within(row).queryByRole("button", { name: ".pixelart" })).toBeNull();
    fireEvent.click(within(row).getByTestId("needs-you-action"));
    expect(props.onFinalizePurchase).toHaveBeenCalledWith(ITEMS[1]);
    expect(props.onOpenName).not.toHaveBeenCalled();
  });

  it("shows Finalizing… for the purchase in flight and disables under a refusal", () => {
    renderCard({ finalizingId: "pur-1" });
    expect(screen.getByRole("button", { name: "Finalizing…" })).toBeDisabled();
  });

  it("disables Finalize when the wallet can't finalize purchases", () => {
    renderCard({ finalizeRefusal: "Unlock to buy" });
    const row = screen.getAllByTestId("needs-you-row")[1]!;
    expect(within(row).getByTestId("needs-you-action")).toBeDisabled();
  });

  it("shows the degraded strip, and no empty line under it", () => {
    renderCard({ items: [], isDegraded: true });
    expect(screen.getByTestId("urgent-tasks-degraded")).toBeInTheDocument();
    expect(screen.queryByTestId("needs-you-empty")).toBeNull();
  });

  it("shows the empty line when nothing is due and the data is healthy", () => {
    renderCard({ items: [] });
    expect(screen.getByTestId("needs-you-empty")).toHaveTextContent("Nothing needs you right now.");
  });

  it("shows a loading line while the first check runs", () => {
    renderCard({ items: [], isLoading: true });
    expect(screen.getByText("Checking your names…")).toBeInTheDocument();
    expect(screen.queryByTestId("needs-you-empty")).toBeNull();
  });
});
