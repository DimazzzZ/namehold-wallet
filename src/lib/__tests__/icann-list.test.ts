import { describe, expect, it } from "vitest";
import list from "../../data/icann-gtld-2026.json";

/** The committed strings list is well formed (spec R1). */
describe("src/data/icann-gtld-2026.json", () => {
  const keys = Object.keys(list.strings);
  const entries = Object.values(list.strings) as {
    applications: { id: string; applicant: string; contention: boolean }[];
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

  it("has variant keys that point at a primary key", () => {
    const variants = Object.entries(list.strings).filter(([, e]) => "variantOf" in e);
    expect(variants.length).toBeGreaterThan(0);
    for (const [, e] of variants) expect(keys).toContain((e as { variantOf: string }).variantOf);
  });

  it("never marks a lone applicant as in contention", () => {
    for (const e of entries) {
      const applicants = new Set(e.applications.map((a) => a.applicant));
      if (applicants.size === 1) {
        expect(e.applications.some((a) => a.contention)).toBe(false);
      }
    }
  });
});
