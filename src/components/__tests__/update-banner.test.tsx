/**
 * Update Banner — "What's new?" modal.
 *
 * Covers the banner's rendering of the "What's new?" button when release
 * notes are available, and the modal that opens on click.
 */
import { describe, it, expect, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";
import { UpdateBanner } from "../UpdateBanner";
import { useAppUpdate } from "../../hooks/useAppUpdate";

beforeEach(() => {
  useAppUpdate.setState({
    phase: "idle",
    available: null,
    progress: null,
    error: null,
    dismissedVersion: null,
  });
});

describe("UpdateBanner — 'What's new?' button", () => {
  it("does not show the button when no update is available", () => {
    render(<UpdateBanner />);
    expect(screen.queryByTestId("update-banner-whats-new")).not.toBeInTheDocument();
  });

  it("does not show the button when notes are empty", () => {
    useAppUpdate.setState({
      phase: "available",
      available: {
        version: "0.4.0",
        currentVersion: "0.3.0",
        notes: null,
        date: null,
      },
    });
    render(<UpdateBanner />);
    expect(screen.queryByTestId("update-banner-whats-new")).not.toBeInTheDocument();
  });

  it("shows the button when notes are available", () => {
    useAppUpdate.setState({
      phase: "available",
      available: {
        version: "0.4.0",
        currentVersion: "0.3.0",
        notes: "## What's Changed\n- Fixed X\n- Added Y",
        date: "2026-07-27",
      },
    });
    render(<UpdateBanner />);
    expect(screen.getByTestId("update-banner-whats-new")).toBeInTheDocument();
  });

  it("opens the modal when the button is clicked", () => {
    useAppUpdate.setState({
      phase: "available",
      available: {
        version: "0.4.0",
        currentVersion: "0.3.0",
        notes: "## What's Changed\n- Fixed X",
        date: "2026-07-27",
      },
    });
    render(<UpdateBanner />);
    expect(screen.queryByTestId("whats-new-modal")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("update-banner-whats-new"));
    expect(screen.getByTestId("whats-new-modal")).toBeInTheDocument();
    expect(screen.getByText(/What's new in v0\.4\.0/)).toBeInTheDocument();
  });

  it("closes the modal when the X button is clicked", () => {
    useAppUpdate.setState({
      phase: "available",
      available: {
        version: "0.4.0",
        currentVersion: "0.3.0",
        notes: "## What's Changed\n- Fixed X",
        date: "2026-07-27",
      },
    });
    render(<UpdateBanner />);
    fireEvent.click(screen.getByTestId("update-banner-whats-new"));
    expect(screen.getByTestId("whats-new-modal")).toBeInTheDocument();
    // Dialog's close button is the × symbol
    const closeBtn = screen.getByRole("button", { name: "×" });
    fireEvent.click(closeBtn);
    expect(screen.queryByTestId("whats-new-modal")).not.toBeInTheDocument();
  });
});
