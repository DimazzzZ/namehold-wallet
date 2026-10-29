import { describe, expect, it, vi } from "vitest";
vi.mock("../../data/icann-gtld-2026.json", async () => ({
  default: (await import("../../test/fixtures/icann-list")).ICANN_FIXTURE,
}));
import { ICANN_FIXTURE as list } from "../../test/fixtures/icann-list";
import {
  ICANN_LIST_FETCHED_AT,
  icannApplicants,
  icannApplicationUrl,
  icannBadgeTitle,
  icannCollision,
  type IcannCollision,
} from "../icann";

const collision = (n: number, contention = false, applicants = n): IcannCollision => ({
  string: "x",
  uLabel: null,
  applications: Array.from({ length: n }, (_, i) => ({
    id: `ID-${i}`,
    applicant: `Applicant ${i % applicants}`,
    contention: contention && i === 0,
  })),
});

describe("icannCollision (spec R2)", () => {
  it("finds an applied-for string by its ACE name", () => {
    const c = icannCollision("agent");
    expect(c?.string).toBe("agent");
    expect(c?.applications).toHaveLength(list.strings.agent.applications.length);
    expect(c?.uLabel).toBeNull();
  });

  it("lowercases before looking up", () => {
    expect(icannCollision("Agent")?.applications).toEqual(list.strings.agent.applications);
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
    const c = icannCollision("xn--3ds831g");
    expect(c?.string).toBe("xn--uirx50m");
    expect(c?.uLabel).toBe("在線");
    expect(c?.applications).toEqual(icannCollision("xn--uirx50m")?.applications);
  });

  it("exposes the list's fetch date", () => {
    expect(ICANN_LIST_FETCHED_AT).toBe(list.fetchedAt);
  });
});

describe("icannApplicants", () => {
  it("groups applications by applicant, first seen first, keeping every id", () => {
    expect(icannApplicants(collision(4, false, 1))).toEqual([
      { applicant: "Applicant 0", ids: ["ID-0", "ID-1", "ID-2", "ID-3"] },
    ]);
    expect(icannApplicants(collision(3)).map((a) => a.applicant)).toEqual([
      "Applicant 0",
      "Applicant 1",
      "Applicant 2",
    ]);
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
  it("counts applicants, not applications", () => {
    expect(icannBadgeTitle(collision(4, false, 1))).toBe(
      "Applied for as a gTLD in ICANN's 2026 round by 1 applicant",
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
