import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { FeeRateOverride, feeRateOverrideIsValid } from "../ui/FeeRateOverride";

describe("FeeRateOverride component", () => {
  it("renders collapsed by default when value is empty", () => {
    const onChange = vi.fn();
    render(<FeeRateOverride value="" onChange={onChange} />);
    // The toggle button exists
    expect(screen.getByTestId("fee-rate-override-toggle")).toBeInTheDocument();
    // Input is not visible (collapsed)
    expect(screen.queryByTestId("fee-rate-override-input")).not.toBeInTheDocument();
  });

  it("renders open when value is non-empty", () => {
    const onChange = vi.fn();
    render(<FeeRateOverride value="5000" onChange={onChange} />);
    expect(screen.getByTestId("fee-rate-override-input")).toBeInTheDocument();
  });

  it("opens on click and shows input", () => {
    const onChange = vi.fn();
    render(<FeeRateOverride value="" onChange={onChange} />);
    fireEvent.click(screen.getByTestId("fee-rate-override-toggle"));
    expect(screen.getByTestId("fee-rate-override-input")).toBeInTheDocument();
  });

  it("shows error for non-integer input", () => {
    const onChange = vi.fn();
    render(<FeeRateOverride value="abc" onChange={onChange} />);
    expect(screen.getByText(/must be a whole number/i)).toBeInTheDocument();
  });

  it("does not show error for valid input", () => {
    const onChange = vi.fn();
    render(<FeeRateOverride value="5000" onChange={onChange} />);
    expect(screen.queryByText(/must be a whole number/i)).not.toBeInTheDocument();
  });

  it("calls onChange when user types", () => {
    const onChange = vi.fn();
    render(<FeeRateOverride value="" onChange={onChange} defaultOpen />);
    const input = screen.getByTestId("fee-rate-override-input");
    fireEvent.change(input, { target: { value: "3000" } });
    expect(onChange).toHaveBeenCalledWith("3000");
  });
});

describe("feeRateOverrideIsValid", () => {
  it("accepts empty (no override)", () => {
    expect(feeRateOverrideIsValid("")).toBe(true);
    expect(feeRateOverrideIsValid("  ")).toBe(true);
  });

  it("accepts valid integer strings", () => {
    expect(feeRateOverrideIsValid("1000")).toBe(true);
    expect(feeRateOverrideIsValid("42000")).toBe(true);
  });

  it("rejects non-integer / non-positive", () => {
    expect(feeRateOverrideIsValid("abc")).toBe(false);
    expect(feeRateOverrideIsValid("1.5")).toBe(false);
    expect(feeRateOverrideIsValid("-1")).toBe(false);
    expect(feeRateOverrideIsValid("0")).toBe(false);
  });
});
