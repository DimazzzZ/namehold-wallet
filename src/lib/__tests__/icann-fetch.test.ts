import { describe, expect, it } from "vitest";
import {
  buildStringsList,
  parseApplicationsCsv,
  parseContentionCsv,
} from "../../../scripts/icann-strings";
import applicationsCsv from "../../test/fixtures/icann-applications.csv?raw";
import contentionCsv from "../../test/fixtures/icann-contention.csv?raw";

const apps = parseApplicationsCsv(applicationsCsv);
const cont = parseContentionCsv(contentionCsv);
const list = buildStringsList(apps, cont, "2026-10-29T12:00:00Z");

describe("the ICANN strings list builder (spec R1)", () => {
  it("keeps the string an applicant ends on", () => {
    expect(list.strings.consult).toBeUndefined();
    expect(list.strings.account!.applications).toEqual([
      { id: "A1-T1-R", applicant: "Consult Co", contention: false },
    ]);
    expect(list.strings.beta!.applications[0]!.id).toBe("B2-T2");
    expect(list.strings.gamma).toBeUndefined();
    expect(list.strings.gone).toBeUndefined();
  });

  it("gives a variant its own key", () => {
    expect(list.strings["xn--uirx50m"]).toMatchObject({ uLabel: "錢包" });
    expect(list.strings["xn--3ds831g"]).toMatchObject({
      uLabel: "在線",
      variantOf: "xn--uirx50m",
    });
    expect(list.strings["xn--3ds831g"]!.applications).toEqual(
      list.strings["xn--uirx50m"]!.applications,
    );
  });

  it("marks contention only where id and string still match", () => {
    expect(list.strings.agent!.applications.map((a) => a.contention)).toEqual([true, true]);
    expect(list.strings["xn--3ds831g"]!.applications[0]!.contention).toBe(true);
    expect(list.strings.account!.applications[0]!.contention).toBe(false);
  });

  it("sorts keys and lowercases them", () => {
    const keys = Object.keys(list.strings);
    expect(keys).toEqual([...keys].sort());
    expect(keys.every((k) => k === k.toLowerCase())).toBe(true);
  });

  it("carries the round, the date and the sources", () => {
    expect(list.round).toBe("2026");
    expect(list.fetchedAt).toBe("2026-10-29T12:00:00Z");
    expect(list.source.applications).toMatch(/^https:\/\/newgtldprogram-aps\.icann\.org\//);
    expect(list.source.contentionSets).toMatch(/^https:\/\/newgtldprogram\.icann\.org\//);
  });

  it("refuses an export whose layout changed", () => {
    expect(() => parseApplicationsCsv("Primary String,Application ID\nagent,X")).toThrow(
      /unexpected ICANN export layout/,
    );
  });

  it("splits a contention cell with two strings", () => {
    expect(cont.filter((c) => c.id === "C3-T3")).toEqual([
      { id: "C3-T3", string: "xn--uirx50m" },
      { id: "C3-T3", string: "xn--3ds831g" },
    ]);
  });
});
