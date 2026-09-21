/**
 * ActionHint — makes a disabled action button's "why" reachable.
 *
 * `Button` sets `disabled:pointer-events-none`, so a disabled button gets no
 * pointer events and a native `title` on it is never rendered by the browser.
 * Every `title={actionReason(...)}` in the name-actions modal was therefore
 * dead exactly when it had something to say, which is why a screenful of
 * disabled buttons looked unexplained.
 */
import { describe, it, expect } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ActionHint } from "../name-actions/ActionHint";
import { Button } from "../ui/Button";

const REASON = "reveal phase not active (phase: 'AVAILABLE')";

describe("ActionHint", () => {
  it("surfaces the reason from a wrapper the pointer can reach", async () => {
    render(
      <ActionHint reason={REASON}>
        <Button disabled>Reveal</Button>
      </ActionHint>,
    );

    const button = screen.getByRole("button", { name: "Reveal" });
    expect(button).toBeDisabled();
    // Never a native title: on a disabled button the browser would not show it.
    expect(button).not.toHaveAttribute("title");

    const trigger = button.parentElement!;
    expect(trigger).not.toHaveClass("pointer-events-none");

    fireEvent.mouseEnter(trigger);
    await waitFor(() => expect(screen.getByRole("tooltip")).toHaveTextContent(REASON));
  });

  it("shows nothing when there is no reason", async () => {
    render(
      <ActionHint reason={null}>
        <Button>Open</Button>
      </ActionHint>,
    );
    fireEvent.mouseEnter(screen.getByRole("button", { name: "Open" }).parentElement!);
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });
});
