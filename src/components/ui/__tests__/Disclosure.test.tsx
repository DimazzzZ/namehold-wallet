import { describe, it, expect } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent } from "@testing-library/react";
import { Disclosure } from "../Disclosure";

describe("Disclosure", () => {
  it("is collapsed by default and sets aria-expanded=false", () => {
    render(
      <Disclosure summary="Toggle me">
        <span>Hidden content</span>
      </Disclosure>,
    );
    const btn = screen.getByRole("button", { name: /Toggle me/i });
    expect(btn).toHaveAttribute("aria-expanded", "false");
  });

  it("always mounts children (present in DOM even when collapsed)", () => {
    render(
      <Disclosure summary="Toggle me">
        <span>Hidden content</span>
      </Disclosure>,
    );
    // textContent is readable even while visually hidden.
    expect(screen.getByText("Hidden content")).toBeInTheDocument();
  });

  it("clicking the toggle opens the disclosure (aria-expanded flips to true)", () => {
    render(
      <Disclosure summary="Toggle me">
        <span>Revealed</span>
      </Disclosure>,
    );
    const btn = screen.getByRole("button", { name: /Toggle me/i });
    fireEvent.click(btn);
    expect(btn).toHaveAttribute("aria-expanded", "true");
  });

  it("respects defaultOpen=true", () => {
    render(
      <Disclosure summary="Open" defaultOpen>
        <span>Visible</span>
      </Disclosure>,
    );
    expect(screen.getByRole("button", { name: /Open/i })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });
});
