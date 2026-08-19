import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { QueryClientProvider, QueryClient } from "@tanstack/react-query";
import { CommandPalette, fuzzyMatch, buildCommands } from "../CommandPalette";

const qc = new QueryClient();

function Harness({
  open,
  onClose,
  route = "/",
}: {
  open: boolean;
  onClose: () => void;
  route?: string;
}) {
  return (
    <QueryClientProvider client={qc}>
      <MemoryRouter initialEntries={[route]}>
        <CommandPalette open={open} onClose={onClose} />
      </MemoryRouter>
    </QueryClientProvider>
  );
}

describe("fuzzyMatch", () => {
  it("matches subsequences case-insensitively", () => {
    expect(fuzzyMatch("sd", "Send")).toBe(true);
    expect(fuzzyMatch("wch", "Watchlist")).toBe(true);
    expect(fuzzyMatch("WLIST", "Watchlist")).toBe(true);
    expect(fuzzyMatch("xyz", "Send")).toBe(false);
  });
});

describe("buildCommands", () => {
  it("includes navigation commands for all PRIMARY_ROUTES", () => {
    const commands = buildCommands(() => {}, "/", true);
    const navCommands = commands.filter((c) => c.category === "Navigation");
    expect(navCommands.length).toBe(6);
  });

  it("includes action commands available on the current route", () => {
    const commands = buildCommands(() => {}, "/", true);
    const actionCommands = commands.filter((c) => c.category === "Actions");
    // On "/" (Wallet): s, r, u, q, / (5 actions)
    expect(actionCommands.length).toBeGreaterThan(0);
    expect(actionCommands.map((c) => c.id)).toContain("wallet:send");
  });

  it("hides write-required actions on read-only wallet", () => {
    const commands = buildCommands(() => {}, "/", false);
    const actionCommands = commands.filter((c) => c.category === "Actions");
    // wallet:send requires write; should be absent.
    expect(actionCommands.map((c) => c.id)).not.toContain("wallet:send");
  });

  it("excludes actions not on the current route", () => {
    const commands = buildCommands(() => {}, "/", true);
    const actionCommands = commands.filter((c) => c.category === "Actions");
    // auctions:batchBid is on /auctions, not /; should be absent.
    expect(actionCommands.map((c) => c.id)).not.toContain("auctions:batchBid");
  });
});

describe("CommandPalette component", () => {
  it("renders when open=true", () => {
    render(<Harness open={true} onClose={() => {}} />);
    expect(screen.getByTestId("command-palette")).toBeInTheDocument();
  });

  it("does not render when open=false", () => {
    render(<Harness open={false} onClose={() => {}} />);
    expect(screen.queryByTestId("command-palette")).not.toBeInTheDocument();
  });

  it("displays all commands when query is empty", () => {
    render(<Harness open={true} onClose={() => {}} />);
    const items = screen.getAllByRole("option");
    expect(items.length).toBeGreaterThan(0);
  });

  it("filters commands by query (subsequence)", () => {
    render(<Harness open={true} onClose={() => {}} />);
    const input = screen.getByTestId("command-palette-input");
    fireEvent.change(input, { target: { value: "wallet" } });
    const items = screen.getAllByRole("option");
    // "Go to Wallet" survives; unrelated commands are filtered out.
    expect(items.some((el) => /Wallet/i.test(el.textContent ?? ""))).toBe(true);
    expect(items.some((el) => /Settings/i.test(el.textContent ?? ""))).toBe(false);
  });

  it("shows 'No results' when nothing matches", () => {
    render(<Harness open={true} onClose={() => {}} />);
    fireEvent.change(screen.getByTestId("command-palette-input"), {
      target: { value: "zzzzzz" },
    });
    expect(screen.getByText("No results")).toBeInTheDocument();
  });

  it("ArrowDown moves selection", () => {
    render(<Harness open={true} onClose={() => {}} />);
    const input = screen.getByTestId("command-palette-input");
    const first = screen.getAllByRole("option")[0]!;
    expect(first.getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(input, { key: "ArrowDown" });
    const options = screen.getAllByRole("option");
    expect(options[0]!.getAttribute("aria-selected")).toBe("false");
    expect(options[1]!.getAttribute("aria-selected")).toBe("true");
  });

  it("Enter runs the selected command and closes", () => {
    const onClose = vi.fn();
    render(<Harness open={true} onClose={onClose} />);
    const input = screen.getByTestId("command-palette-input");
    // First command on "/" is "Go to Wallet" (navigation). Running it closes.
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onClose).toHaveBeenCalled();
  });

  it("Escape closes the palette", () => {
    const onClose = vi.fn();
    render(<Harness open={true} onClose={onClose} />);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("dispatches an action when an action command runs", () => {
    const onClose = vi.fn();
    const events: string[] = [];
    const listener = (e: Event) =>
      events.push((e as CustomEvent).detail.actionId);
    window.addEventListener("namehold:action", listener);
    render(<Harness open={true} onClose={onClose} />);
    // Filter to the Sync action and run it.
    fireEvent.change(screen.getByTestId("command-palette-input"), {
      target: { value: "Sync" },
    });
    fireEvent.keyDown(screen.getByTestId("command-palette-input"), { key: "Enter" });
    expect(events).toContain("wallet:sync");
    window.removeEventListener("namehold:action", listener);
  });
});
