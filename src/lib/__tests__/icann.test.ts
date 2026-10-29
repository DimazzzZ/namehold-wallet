import { describe, expect, it } from "vitest";
import list from "../../data/icann-gtld-2026.json";
import {
  ICANN_LIST_FETCHED_AT,
  icannApplicationUrl,
  icannBadgeTitle,
  icannCollision,
  type IcannCollision,
} from "../icann";

const collision = (n: number, contention = false): IcannCollision => ({
  string: "x",
  uLabel: null,
  applications: Array.from({ length: n }, (_, i) => ({
    id: `ID-${i}`,
    applicant: `Applicant ${i}`,
    contention: contention && i === 0,
  })),
});

describe("icannCollision (spec R2)", () => {
  it("finds an applied-for string by its ACE name", () => {
    const c = icannCollision("agent");
    expect(c?.string).toBe("agent");
    expect(c?.applications).toHaveLength(11);
    expect(c?.uLabel).toBeNull();
  });

  it("lowercases before looking up", () => {
    expect(icannCollision("Agent")?.applications).toHaveLength(11);
  });

  it("finds an IDN by its A-label and carries its Unicode label", () => {
    const c = icannCollision("xn--3ds443g");
    expect(c?.uLabel).toBe("在线");
  });

  it("never matches a decoded (Unicode) form", () => {
    expect(icannCollision("在线")).toBeNull();
  });

  it("answers null for a name nobody applied for", () => {
    expect(icannCollision("zzzz-not-applied")).toBeNull();
  });

  it("resolves a variant key to its primary string", () => {
    const [key, entry] = Object.entries(list.strings).find(([, e]) => "variantOf" in e) as [
      string,
      { variantOf: string },
    ];
    const c = icannCollision(key);
    expect(c?.string).toBe(entry.variantOf);
    expect(c?.applications).toEqual(icannCollision(entry.variantOf)?.applications);
  });

  it("exposes the list's fetch date", () => {
    expect(ICANN_LIST_FETCHED_AT).toBe(list.fetchedAt);
  });
});

describe("icannBadgeTitle", () => {
  it("counts one applicant", () => {
    expect(icannBadgeTitle(collision(1))).toBe(
      "Applied for as a gTLD in ICANN's 2026 round by 1 applicant",
    );
  });
  it("counts several applicants", () => {
    expect(icannBadgeTitle(collision(2))).toBe(
      "Applied for as a gTLD in ICANN's 2026 round by 2 applicants",
    );
  });
  it("says when the string is in contention", () => {
    expect(icannBadgeTitle(collision(2, true))).toBe(
      "Applied for as a gTLD in ICANN's 2026 round by 2 applicants · in contention",
    );
  });
});

describe("icannApplicationUrl", () => {
  it("points at the application's own page", () => {
    expect(icannApplicationUrl("AA2634T-T39423")).toBe(
      "https://newgtldprogram-aps.icann.org/applications/AA2634T-T39423/summary",
    );
  });
});
