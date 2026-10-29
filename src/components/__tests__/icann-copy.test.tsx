import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { NameDetails } from "../name-actions/NameDetails";
import { ICANN_LABELS } from "../names/NameFiltersPopover";
import { ICANN_HINT, icannBadgeTitle, icannCollision } from "../../lib/icann";

vi.mock("@tauri-apps/api/core", () => ({ invoke: () => Promise.resolve(null) }));

// Spec R8: the flag states a fact. Nothing in its copy predicts delegation or
// calls the name taken or lost.
const FORBIDDEN = /will be delegated|\btaken\b|\blost\b/i;

describe("ICANN collision copy (spec R8)", () => {
  const agent = icannCollision("agent")!;

  it("badge tooltip, hint and filter labels", () => {
    expect(icannBadgeTitle(agent)).not.toMatch(FORBIDDEN);
    expect(ICANN_HINT).not.toMatch(FORBIDDEN);
    expect(ICANN_HINT).toMatch(/^Someone applied for this string/);
    for (const label of Object.values(ICANN_LABELS)) expect(label).not.toMatch(FORBIDDEN);
  });

  it("the Details row as rendered", () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={qc}>
        <NameDetails name="agent" profileId="p1" info={null} />
      </QueryClientProvider>,
    );
    expect(screen.getByTestId("name-icann").textContent).not.toMatch(FORBIDDEN);
  });
});
