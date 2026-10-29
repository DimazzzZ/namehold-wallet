import { describe, it, expect } from "vitest";
import { formatSince } from "../utils";

const NOW = Date.parse("2026-10-29T12:00:00Z");
const ago = (ms: number) => new Date(NOW - ms).toISOString();

describe("formatSince", () => {
  it("is relative under 24 h", () => {
    expect(formatSince(ago(30_000), NOW)).toBe("just now");
    expect(formatSince(ago(14 * 60_000), NOW)).toBe("14 min ago");
    expect(formatSince(ago(3 * 3_600_000 + 59 * 60_000), NOW)).toBe("3 h ago");
  });

  it("switches to the UTC date at 24 h", () => {
    expect(formatSince(ago(24 * 3_600_000 - 60_000), NOW)).toBe("23 h ago");
    expect(formatSince("2026-10-27T13:05:00Z", NOW)).toBe("Oct 27 · 13:05");
  });

  it("treats a clock-skewed future timestamp as just now and empty input as a dash", () => {
    expect(formatSince(ago(-5 * 60_000), NOW)).toBe("just now");
    expect(formatSince(null, NOW)).toBe("—");
  });
});
