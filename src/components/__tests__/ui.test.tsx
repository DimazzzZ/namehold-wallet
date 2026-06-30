import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import { Input } from "../ui/Input";
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
