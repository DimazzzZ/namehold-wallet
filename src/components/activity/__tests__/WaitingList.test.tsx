import { afterEach, describe, it, expect, vi } from "vitest";
import "@testing-library/jest-dom";
import { act, render, screen } from "@testing-library/react";
import type { MergedRow } from "../../../lib/activity";
import { WaitingList } from "../WaitingList";

afterEach(() => vi.useRealTimers());

describe("WaitingList", () => {
  it("shows time since sent and moves it on every minute", () => {
    vi.useFakeTimers();
    const now = Date.parse("2026-10-29T12:00:00Z");
    vi.setSystemTime(now);
    // An incoming payment in the mempool: no draft, so no Speed up to wire up.
    const row = {
      key: "r1",
      action: "send",
      valueDoos: 1_000_000,
      status: "broadcasted",
      sortTs: (now - 14 * 60_000) / 1000,
      draftId: null,
      txid: null,
      name: null,
      nameList: null,
    } as unknown as MergedRow;
    render(<WaitingList rows={[row]} onNameClick={vi.fn()} onTxClick={vi.fn()} />);
    expect(screen.getByTestId("activity-waiting-sent")).toHaveTextContent("sent 14 min ago");
    act(() => void vi.advanceTimersByTime(60_000));
    expect(screen.getByTestId("activity-waiting-sent")).toHaveTextContent("sent 15 min ago");
  });
});
