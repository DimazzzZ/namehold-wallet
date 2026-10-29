import { describe, expect, it } from "vitest";
import list from "../../data/icann-gtld-2026.json";

/** The committed strings list is well formed (spec R1). */
describe("src/data/icann-gtld-2026.json", () => {
  const keys = Object.keys(list.strings);
  const entries = Object.values(list.strings) as {
    applications: { id: string; applicant: string }[];
  }[];

  it("keys are lowercase ACE labels, sorted", () => {
    expect(keys.length).toBeGreaterThan(1000);
    expect(keys.every((k) => /^[a-z0-9-]+$/.test(k))).toBe(true);
    expect(keys).toEqual([...keys].sort());
  });

  it("every application has an id and an applicant", () => {
    for (const e of entries) {
      expect(e.applications.length).toBeGreaterThan(0);
      for (const a of e.applications) {
        expect(a.id).toMatch(/\S/);
        expect(a.applicant).toMatch(/\S/);
      }
    }
  });

  it("carries a parseable fetch date and the 2026 round", () => {
    expect(list.round).toBe("2026");
    expect(Number.isNaN(Date.parse(list.fetchedAt))).toBe(false);
  });

  it("holds agent and not a string its applicant switched away from", () => {
    expect(keys).toContain("agent");
    expect(keys).not.toContain("action");
  });
});
