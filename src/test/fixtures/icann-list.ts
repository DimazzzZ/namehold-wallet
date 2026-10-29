/**
 * A small ICANN strings list for tests, in the shape of
 * `src/data/icann-gtld-2026.json`. Tests mock the data module with it so a
 * refresh of the committed list (spec R10) never touches a test:
 *
 *   vi.mock("../../data/icann-gtld-2026.json", async () => ({
 *     default: (await import("../../test/fixtures/icann-list")).ICANN_FIXTURE,
 *   }));
 *
 * - `agent`: 11 applications by 11 applicants, the first two in contention.
 * - `apex`: 4 applications by one applicant (spec R4/R6 count applicants).
 * - `xn--3ds443g`: an IDN with its Unicode label.
 * - `xn--uirx50m` with the variant `xn--3ds831g` (`variantOf`).
 */
const app = (id: string, applicant: string, contention = false) => ({ id, applicant, contention });

export const ICANN_FIXTURE = {
  round: "2026" as const,
  fetchedAt: "2026-10-29T12:00:00.000Z",
  source: {
    applications: "https://newgtldprogram-aps.icann.org/aps/applications/export/csv",
    contentionSets:
      "https://newgtldprogram.icann.org/en/application-rounds/round2/reveal-day-contention-sets.csv",
  },
  strings: {
    agent: {
      applications: Array.from({ length: 11 }, (_, i) =>
        app(`AG${i + 1}-T${i + 1}`, `Agent Applicant ${i + 1}`, i < 2),
      ),
    },
    apex: {
      applications: [
        app("AP1-T1", "Apex Co"),
        app("AP1-T2", "Apex Co"),
        app("AP1-T3", "Apex Co"),
        app("AP1-T4", "Apex Co"),
      ],
    },
    "xn--3ds443g": { uLabel: "在线", applications: [app("ON1-T1", "Online Ltd")] },
    "xn--3ds831g": {
      uLabel: "在線",
      variantOf: "xn--uirx50m",
      applications: [app("WA1-T1", "Wallet Inc")],
    },
    "xn--uirx50m": { uLabel: "錢包", applications: [app("WA1-T1", "Wallet Inc")] },
  },
};

/** The applicants of `agent` in the fixture, for assertions. */
export const AGENT_APPLICANTS = ICANN_FIXTURE.strings.agent.applications.map((a) => a.applicant);
