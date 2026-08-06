import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { Input, inputSizes } from "../ui/Input";
import { Select } from "../ui/Select";
import { Dialog } from "../ui/Dialog";
import { StatusBadge } from "../ui/StatusBadge";
import type { MigrationStatus } from "../../types";

describe("Button", () => {
  it("renders children text", () => {
    render(<Button>Click me</Button>);
    expect(screen.getByText("Click me")).toBeTruthy();
  });

  it("renders with primary variant", () => {
    render(<Button variant="primary">Primary</Button>);
    expect(screen.getByText("Primary")).toBeTruthy();
  });

  it("renders with danger variant", () => {
    render(<Button variant="danger">Danger</Button>);
    expect(screen.getByText("Danger")).toBeTruthy();
  });

  it("renders disabled state", () => {
    render(<Button disabled>Disabled</Button>);
    const btn = screen.getByText("Disabled") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("renders small size", () => {
    render(<Button size="sm">Small</Button>);
    expect(screen.getByText("Small")).toBeTruthy();
  });

  it("calls onClick handler", () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Click</Button>);
    screen.getByText("Click").click();
    expect(onClick).toHaveBeenCalledOnce();
  });
});

describe("Badge", () => {
  it("renders text", () => {
    render(<Badge>Test</Badge>);
    expect(screen.getByText("Test")).toBeTruthy();
  });

  it("renders with success variant", () => {
    render(<Badge variant="success">OK</Badge>);
    expect(screen.getByText("OK")).toBeTruthy();
  });

  it("renders with error variant", () => {
    render(<Badge variant="error">Fail</Badge>);
    expect(screen.getByText("Fail")).toBeTruthy();
  });
});

describe("Input", () => {
  it("renders with label", () => {
    render(<Input label="Name" />);
    expect(screen.getByLabelText("Name")).toBeTruthy();
  });

  it("renders with placeholder", () => {
    render(<Input placeholder="Enter value" />);
    expect(screen.getByPlaceholderText("Enter value")).toBeTruthy();
  });

  it("renders with type password", () => {
    render(<Input type="password" label="Pass" />);
    const input = screen.getByLabelText("Pass") as HTMLInputElement;
    expect(input.type).toBe("password");
  });

  it("applies the sm size tokens", () => {
    render(<Input inputSize="sm" placeholder="Small" />);
    const el = screen.getByPlaceholderText("Small");
    // sm → px-2.5 py-1 text-xs (matches Button sm)
    expect(el.className).toContain("px-2.5");
    expect(el.className).toContain("py-1");
    expect(el.className).toContain("text-xs");
  });

  it("applies the md size tokens by default", () => {
    render(<Input placeholder="Medium" />);
    const el = screen.getByPlaceholderText("Medium");
    // md → px-3 py-1.5 text-sm
    expect(el.className).toContain("px-3");
    expect(el.className).toContain("py-1.5");
    expect(el.className).toContain("text-sm");
  });
});

describe("Control height alignment (Input / Select / Button share size tokens)", () => {
  // The whole point of the sizing system: a control and its adjacent button
  // at the same `size` must carry the SAME vertical-padding + text-size
  // tokens, so they render at identical heights. Button also gets a
  // (transparent) border at every variant so the border box matches too.
  it("Input md and Button md share py + text tokens", () => {
    render(
      <>
        <Input placeholder="field" />
        <Button size="md">Go</Button>
      </>,
    );
    const input = screen.getByPlaceholderText("field");
    const button = screen.getByText("Go");
    // Both md: py-1.5 + text-sm.
    expect(input.className).toContain("py-1.5");
    expect(input.className).toContain("text-sm");
    expect(button.className).toContain("py-1.5");
    expect(button.className).toContain("text-sm");
  });

  it("Input sm and Button sm share py + text tokens", () => {
    render(
      <>
        <Input inputSize="sm" placeholder="field" />
        <Button size="sm">Go</Button>
      </>,
    );
    const input = screen.getByPlaceholderText("field");
    const button = screen.getByText("Go");
    expect(input.className).toContain("py-1");
    expect(input.className).toContain("text-xs");
    expect(button.className).toContain("py-1");
    expect(button.className).toContain("text-xs");
  });

  it("Select shares the same size tokens as Input", () => {
    render(<Select options={[{ value: "a", label: "A" }]} label="Pick" />);
    const select = screen.getByLabelText("Pick");
    expect(select.className).toContain(inputSizes.md.split(" ")[0]); // px-3
    expect(select.className).toContain("py-1.5");
    expect(select.className).toContain("text-sm");
  });

  it("all Button variants carry a border so heights match bordered inputs", () => {
    render(
      <>
        <Button variant="primary">P</Button>
        <Button variant="secondary">S</Button>
        <Button variant="danger">D</Button>
        <Button variant="ghost">G</Button>
      </>,
    );
    for (const label of ["P", "S", "D", "G"]) {
      expect(screen.getByText(label).className).toContain("border");
    }
  });
});

describe("Dialog", () => {
  it("renders when open", () => {
    render(
      <Dialog open={true} onClose={() => {}} title="Test Dialog">
        <p>Content</p>
      </Dialog>
    );
    expect(screen.getByText("Test Dialog")).toBeTruthy();
    expect(screen.getByText("Content")).toBeTruthy();
  });

  it("does not render when closed", () => {
    render(
      <Dialog open={false} onClose={() => {}} title="Test Dialog">
        <p>Content</p>
      </Dialog>
    );
    expect(screen.queryByText("Test Dialog")).toBeNull();
  });
});

describe("StatusBadge", () => {
  const statuses: MigrationStatus[] = [
    "not_started",
    "namebase_transfer_requested",
    "waiting_transfer_tx",
    "transfer_seen_on_chain",
    "waiting_finalize",
    "finalized_owned",
    "failed_or_stuck",
    "do_not_touch_staked",
  ];

  statuses.forEach((status) => {
    it(`renders ${status}`, () => {
      render(<StatusBadge status={status} />);
      expect(screen.getByText(/.+/)).toBeTruthy();
    });
  });

  it("shows correct label for finalized_owned", () => {
    render(<StatusBadge status="finalized_owned" />);
    expect(screen.getByText("Finalized")).toBeTruthy();
  });

  it("shows correct label for do_not_touch_staked", () => {
    render(<StatusBadge status="do_not_touch_staked" />);
    expect(screen.getByText("Do Not Touch")).toBeTruthy();
  });

  it("shows correct label for failed_or_stuck", () => {
    render(<StatusBadge status="failed_or_stuck" />);
    expect(screen.getByText("Failed/Stuck")).toBeTruthy();
  });
});
