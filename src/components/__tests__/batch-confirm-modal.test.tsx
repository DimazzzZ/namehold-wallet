import { describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { BatchConfirmModal } from "../BatchConfirmModal";

// The fee is passed in doos and shown in HNS once: it was divided by a
// million twice and read ~0.000000 HNS for every batch.
describe("BatchConfirmModal fee and amount", () => {
  const base = {
    open: true,
    action: "renew" as const,
    names: ["a", "b"],
    amountDoos: 2_000_000,
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
  };

  it("shows the fee of a single transaction in HNS", () => {
    render(<BatchConfirmModal {...base} estimatedFeeDoos={15_933} />);
    const text = screen.getByRole("dialog").textContent ?? "";
    expect(text).toMatch(/Estimated fee:\s*~0\.015933\s*HNS/);
    expect(text).not.toMatch(/for all transactions|first of/);
  });

  it("a batch bid's fee is not divided twice either", () => {
    render(<BatchConfirmModal {...base} action="bid" estimatedFeeDoos={2_620} />);
    expect(screen.getByRole("dialog").textContent).toMatch(/~0\.002620\s*HNS/);
  });

  it("a series says the fee covers every transaction and the amount is the first one's", () => {
    render(<BatchConfirmModal {...base} batches={3} estimatedFeeDoos={47_799} />);
    const text = screen.getByRole("dialog").textContent ?? "";
    expect(text).toMatch(/~0\.047799\s*HNS for all transactions/);
    expect(screen.getByTestId("batch-amount")).toHaveTextContent("in the first of 3 transactions");
  });
});
