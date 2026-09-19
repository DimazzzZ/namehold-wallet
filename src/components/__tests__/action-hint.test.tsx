/**
 * ActionHint — makes a disabled action button's "why" reachable.
 *
 * `Button` sets `disabled:pointer-events-none`, so a disabled button gets no
 * pointer events and the browser never renders its `title`. Every
 * `title={actionReason(...)}` in the name-actions modal was therefore dead
 * exactly when it had something to say, which is why a screenful of disabled
 * buttons looked unexplained.
 */
import { describe, it, expect } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { ActionHint } from "../name-actions/ActionHint";
import { Button } from "../ui/Button";

describe("ActionHint", () => {
  it("carries the reason on an element the pointer can reach", () => {
    render(
      <ActionHint reason="reveal phase not active (phase: 'AVAILABLE')">
        <Button disabled>Reveal</Button>
      </ActionHint>,
    );

    const button = screen.getByRole("button", { name: "Reveal" });
    expect(button).toBeDisabled();

    // The title must NOT be on the button: `disabled:pointer-events-none`
    // means the browser would never surface it there.
    expect(button).not.toHaveAttribute("title");

    const hint = button.parentElement;
    expect(hint).toHaveAttribute("title", "reveal phase not active (phase: 'AVAILABLE')");
    // The wrapper must not swallow pointer events itself.
    expect(hint).not.toHaveClass("pointer-events-none");
  });

  it("adds no wrapper when there is no reason", () => {
    const { container } = render(
      <ActionHint reason={null}>
        <Button>Open</Button>
      </ActionHint>,
    );
    expect(container.firstChild).toBe(screen.getByRole("button", { name: "Open" }));
  });
});
