import { describe, it, expect, vi } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { BidGate } from "../BidGate";
import type { PhaseCountdown } from "../../../lib/auction";
import type { NameActionCapability } from "../../../types";

/**
 * Regression: the advanced auction section used to render <BidForm/>
 * unconditionally, so Bid / Lockup inputs were visible in phases where
 * bidding is impossible (AVAILABLE / OPENING / BIDDING-after-bid). The
 * shared BidGate replaces that: a single `canBid.allowed` predicate decides
 * whether the fields appear, with a contextual placeholder (using the
 * existing countdown) when they do not.
 */

const baseFormProps = {
  variant: "advanced" as const,
  bidHns: "",
  onBidChange: vi.fn(),
  lockupHns: "",
  onLockupChange: vi.fn(),
  bidError: null,
  lockupError: null,
  forfeitLockupText: "0",
  disabled: false,
  busy: false,
  onSubmit: vi.fn(),
  idleLabel: "Bid",
  busyLabel: "…",
};

const allowed: NameActionCapability = { allowed: true, reason: null };
const denied = (reason: string | null = null): NameActionCapability => ({
  allowed: false,
  reason,
});

const openingCountdown: PhaseCountdown = {
  label: "Bidding opens in",
  blocks: 12,
  hours: 2,
};
const revealCountdown: PhaseCountdown = {
  label: "Reveal starts in",
  blocks: 6,
  hours: 1,
};

describe("BidGate", () => {
  it("renders the BidForm fields when canBid.allowed is true", () => {
    render(
      <BidGate {...baseFormProps} canBid={allowed} phase="BIDDING" countdown={revealCountdown} />,
    );
    // BidForm inputs are labelled by <Input label="Bid (HNS)">/"Lockup (HNS)".
    expect(screen.getByLabelText("Bid (HNS)")).toBeInTheDocument();
    expect(screen.getByLabelText("Lockup (HNS)")).toBeInTheDocument();
    expect(screen.queryByTestId("bid-gate-placeholder")).not.toBeInTheDocument();
  });

  it("hides fields and shows the AVAILABLE placeholder when canBid is denied", () => {
    render(
      <BidGate
        {...baseFormProps}
        canBid={denied("Phase is AVAILABLE")}
        phase="AVAILABLE"
        countdown={null}
      />,
    );
    expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
    const ph = screen.getByTestId("bid-gate-placeholder");
    expect(ph).toHaveTextContent(/open the auction first/i);
  });

  it("shows the OPENING placeholder with the formatted countdown", () => {
    render(
      <BidGate
        {...baseFormProps}
        canBid={denied("Auction is opening")}
        phase="OPENING"
        countdown={openingCountdown}
      />,
    );
    expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
    // formatCountdown(openingCountdown) → "12 blocks (~2h)"
    expect(screen.getByTestId("bid-gate-placeholder")).toHaveTextContent(
      /Bidding opens in 12 blocks \(~2h\)/i,
    );
  });

  it("shows the hasBid placeholder in BIDDING when canBid is denied", () => {
    // canBid.allowed is false in BIDDING only once the user has already bid.
    render(
      <BidGate
        {...baseFormProps}
        canBid={denied("Already bid")}
        phase="BIDDING"
        countdown={revealCountdown}
      />,
    );
    expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
    expect(screen.getByTestId("bid-gate-placeholder")).toHaveTextContent(
      /Your bid is placed\. Reveal opens in 6 blocks \(~1h\)/i,
    );
  });

  it("falls back to canBid.reason when no countdown is available", () => {
    render(
      <BidGate
        {...baseFormProps}
        canBid={denied("Node not synced")}
        phase="OPENING"
        countdown={null}
      />,
    );
    const ph = screen.getByTestId("bid-gate-placeholder");
    expect(ph).toHaveTextContent(/Bidding opens after the opening period/i);
    // Fallback reason is surfaced beneath the generic copy.
    expect(ph).toHaveTextContent(/Node not synced/i);
  });

  it("renders nothing outside pre-bidding phases (REVEAL / CLOSED / TRANSFER / REVOKED)", () => {
    for (const phase of ["REVEAL", "CLOSED", "TRANSFER", "REVOKED"] as const) {
      const { container, unmount } = render(
        <BidGate {...baseFormProps} canBid={denied("N/A")} phase={phase} countdown={null} />,
      );
      expect(container.textContent ?? "").toBe("");
      expect(screen.queryByLabelText("Bid (HNS)")).not.toBeInTheDocument();
      unmount();
    }
  });
});
