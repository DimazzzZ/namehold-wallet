import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { StatusBadge } from "../ui/StatusBadge";
import { Button } from "../ui/Button";
import { Badge } from "../ui/Badge";
import type { MigrationStatus } from "../../types";

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
    it(`renders status: ${status}`, () => {
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
});

describe("Button", () => {
  it("renders children", () => {
    render(<Button>Click me</Button>);
    expect(screen.getByText("Click me")).toBeTruthy();
  });
  it("renders with variant", () => {
    render(<Button variant="primary">Primary</Button>);
    expect(screen.getByText("Primary")).toBeTruthy();
  });
  it("renders disabled", () => {
    render(<Button disabled>Disabled</Button>);
    const btn = screen.getByText("Disabled") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });
});

describe("Badge", () => {
  it("renders children", () => {
    render(<Badge>Test</Badge>);
    expect(screen.getByText("Test")).toBeTruthy();
  });
  it("renders with variant", () => {
    render(<Badge variant="success">Success</Badge>);
    expect(screen.getByText("Success")).toBeTruthy();
  });
});
