/**
 * Tooltip — the app's single hover-hint primitive, replacing native `title`.
 */
import { describe, it, expect, vi, afterEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { Tooltip, TOOLTIP_OPEN_DELAY_MS } from "../ui/Tooltip";
import { Button } from "../ui/Button";

afterEach(() => vi.useRealTimers());

describe("Tooltip", () => {
  it("waits out the hover debounce before appearing, then closes at once", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    render(
      <Tooltip content="Freshly mined coins — spendable once they mature">
        <span>Immature</span>
      </Tooltip>,
    );
    const trigger = screen.getByText("Immature").parentElement!;

    fireEvent.mouseEnter(trigger);
    // A pointer merely sweeping across must not flash a tooltip.
    act(() => void vi.advanceTimersByTime(TOOLTIP_OPEN_DELAY_MS - 50));
    expect(screen.queryByRole("tooltip")).toBeNull();

    act(() => void vi.advanceTimersByTime(60));
    expect(screen.getByRole("tooltip")).toHaveTextContent("Freshly mined coins");

    // Closing is immediate — a tooltip that lingers reads as stuck.
    fireEvent.mouseLeave(trigger);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("shows on keyboard focus, with no debounce to wait through", async () => {
    render(
      <Tooltip content="Show QR code">
        <button type="button">QR</button>
      </Tooltip>,
    );
    fireEvent.focus(screen.getByRole("button", { name: "QR" }));
    await waitFor(() => expect(screen.getByRole("tooltip")).toHaveTextContent("Show QR code"));
  });

  it("works on a disabled control, where a native title never would", async () => {
    // `Button` sets `disabled:pointer-events-none`; the wrapper still listens.
    render(
      <Tooltip content="All selected names must be in the REVEAL phase" openDelay={0}>
        <Button disabled>Reveal Selected</Button>
      </Tooltip>,
    );
    const button = screen.getByRole("button", { name: "Reveal Selected" });
    expect(button).toBeDisabled();

    fireEvent.mouseEnter(button.parentElement!);
    await waitFor(() =>
      expect(screen.getByRole("tooltip")).toHaveTextContent("must be in the REVEAL phase"),
    );
  });

  it("shows nothing when there is nothing to say", async () => {
    render(
      <Tooltip content={null} openDelay={0}>
        <button type="button">Open</button>
      </Tooltip>,
    );
    const button = screen.getByRole("button", { name: "Open" });
    fireEvent.mouseEnter(button.parentElement!);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it("keeps the same DOM node when the content appears and disappears", () => {
    // Remounting on every content change would drop focus and caret position
    // in anything interactive, and hand stale nodes to anyone holding a ref.
    const { rerender } = render(
      <Tooltip content="Enter a recipient address">
        <button type="button">Transfer</button>
      </Tooltip>,
    );
    const before = screen.getByRole("button", { name: "Transfer" });

    rerender(
      <Tooltip content={undefined}>
        <button type="button">Transfer</button>
      </Tooltip>,
    );
    expect(screen.getByRole("button", { name: "Transfer" })).toBe(before);
  });

  it("only underlines the trigger when asked to hint at it", () => {
    const { rerender } = render(
      <Tooltip content="x">
        <span>plain</span>
      </Tooltip>,
    );
    expect(screen.getByText("plain").parentElement).not.toHaveClass("underline");

    rerender(
      <Tooltip content="x" hint>
        <span>plain</span>
      </Tooltip>,
    );
    expect(screen.getByText("plain").parentElement).toHaveClass("underline");
  });
});
