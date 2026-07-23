import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const writeTextMock = vi.fn().mockResolvedValue(undefined);
vi.mock("../../../lib/clipboard", () => ({
  writeText: (...args: unknown[]) => writeTextMock(...args),
  readText: vi.fn().mockResolvedValue(""),
}));

import { CopyField } from "../CopyField";

beforeEach(() => {
  writeTextMock.mockClear();
});

describe("CopyField", () => {
  it("renders the full value into the value node", () => {
    render(<CopyField label="Xpub" value="the-full-value" valueTestId="v" />);
    expect(screen.getByTestId("v")).toHaveTextContent("the-full-value");
    expect(screen.getByText("Xpub")).toBeInTheDocument();
  });

  it("shows the display string but copies the full value", async () => {
    render(
      <CopyField
        value="the-full-value"
        display="the…value"
        valueTestId="v"
        copyTestId="c"
      />,
    );
    expect(screen.getByTestId("v")).toHaveTextContent("the…value");
    fireEvent.click(screen.getByTestId("c"));
    await waitFor(() => {
      expect(writeTextMock).toHaveBeenCalledWith("the-full-value");
    });
  });

  it("flips the button label to 'Copied!' after a copy", async () => {
    render(<CopyField value="x" copyLabel="Copy public key" copyTestId="c" />);
    const btn = screen.getByTestId("c");
    expect(btn).toHaveTextContent("Copy public key");
    fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveTextContent("Copied!"));
  });

  it("disables the copy button when value is empty", () => {
    render(<CopyField value="" copyTestId="c" />);
    expect(screen.getByTestId("c")).toBeDisabled();
  });
});
