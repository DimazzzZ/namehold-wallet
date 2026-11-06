import { describe, it, expect, vi, beforeEach } from "vitest";
import "@testing-library/jest-dom";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const invokeMock = vi.fn();
vi.mock("../../lib/invoke", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

import { StartupGate } from "../StartupGate";

const app = <div>the app</div>;

describe("StartupGate", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("shows the app at once when the database is ready", async () => {
    invokeMock.mockResolvedValue({ state: "ready" });
    render(<StartupGate pollMs={10}>{app}</StartupGate>);
    expect(await screen.findByText("the app")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("startup_status");
  });

  it("shows the migration's progress, then the app", async () => {
    invokeMock.mockResolvedValue({
      state: "migrating",
      step: 1,
      total: 2,
      version: "042",
      elapsed_secs: 75,
      percent: 34,
    });
    render(<StartupGate pollMs={10}>{app}</StartupGate>);

    expect(await screen.findByText("Updating Namehold's database")).toBeInTheDocument();
    expect(screen.getByText(/Step 1 of 2/)).toBeInTheDocument();
    expect(screen.getByText(/34%/)).toBeInTheDocument();
    expect(screen.getByText(/1 min 15 s/)).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "34");
    expect(screen.getByText(/Keep Namehold open/)).toBeInTheDocument();
    expect(screen.queryByText("the app")).not.toBeInTheDocument();

    // The migration finishes: the next poll brings the app up.
    invokeMock.mockResolvedValue({ state: "ready" });
    expect(await screen.findByText("the app")).toBeInTheDocument();
  });

  it("shows a failure with its reason and retries on request", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "startup_status" ? { state: "failed", error: "database is locked" } : null,
    );
    render(<StartupGate pollMs={10}>{app}</StartupGate>);

    expect(await screen.findByText(/database is locked/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("retry_startup"));
    expect(screen.queryByText("the app")).not.toBeInTheDocument();
  });
});
