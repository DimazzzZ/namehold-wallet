import { describe, it, expect, vi } from "vitest";
import "@testing-library/jest-dom";
import { render, screen } from "@testing-library/react";
import { Tabs } from "../Tabs";

describe("Tabs", () => {
  it("marks the active tab gray-900 semibold with a gray-900 line, never blue", () => {
    render(
      <Tabs
        tabs={[
          { key: "a", label: "Alpha" },
          { key: "b", label: "Bravo" },
        ]}
        active="a"
        onChange={vi.fn()}
      />,
    );
    const active = screen.getByRole("tab", { name: "Alpha" });
    const inactive = screen.getByRole("tab", { name: "Bravo" });
    expect(active).toHaveClass("text-gray-900", "font-semibold", "border-gray-900");
    expect(inactive).toHaveClass("text-gray-600", "hover:text-gray-900", "border-transparent");
    expect(active.className).not.toMatch(/blue-(600|700)/);
  });
});
