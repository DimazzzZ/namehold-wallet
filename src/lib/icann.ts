/**
 * ICANN collision flag (docs/specs/2026-10-29-icann-collision-flag.md).
 *
 * Answers whether a Handshake name is a string someone applied for as a new
 * gTLD in ICANN's 2026 round, from the committed list
 * `src/data/icann-gtld-2026.json` (refreshed by `just icann-strings`; the
 * app never asks ICANN at run time).
 *
 * R2: the comparison is the lowercased ACE name against the list's A-label
 * keys. The decoded display form of an IDN (`displayName` in `idn.ts`) is
 * never compared and must never be passed here.
 */
import list from "../data/icann-gtld-2026.json";

export interface IcannApplication {
  id: string;
  applicant: string;
  /** In a Reveal Day contention set (more than one applicant for the string). */
  contention: boolean;
}

export interface IcannCollision {
  /** The applied-for string as an A-label (the primary string for a variant). */
  string: string;
  /** The Unicode form of the looked-up label, when ICANN gives one. */
  uLabel: string | null;
  applications: IcannApplication[];
}

interface IcannEntry {
  uLabel?: string;
  variantOf?: string;
  applications: IcannApplication[];
}

const strings = list.strings as Record<string, IcannEntry | undefined>;

/** When the committed list was fetched from ICANN (ISO date). */
export const ICANN_LIST_FETCHED_AT: string = list.fetchedAt;

/** The applied-for string a name collides with, or null. */
export function icannCollision(name: string): IcannCollision | null {
  const key = name.toLowerCase();
  const entry = strings[key];
  if (!entry) return null;
  return {
    string: entry.variantOf ?? key,
    uLabel: entry.uLabel ?? null,
    applications: entry.applications,
  };
}

/** The application's own page on ICANN's Application Publication site. */
export function icannApplicationUrl(id: string): string {
  return `https://newgtldprogram-aps.icann.org/applications/${encodeURIComponent(id)}/summary`;
}

/** The badge tooltip (R4, R5): a fact about the round, never a prediction (R8). */
export function icannBadgeTitle(c: IcannCollision): string {
  const n = c.applications.length;
  const base = `Applied for as a gTLD in ICANN's 2026 round by ${n} applicant${n === 1 ? "" : "s"}`;
  return c.applications.some((a) => a.contention) ? `${base} · in contention` : base;
}

/** The one sentence that names a consequence, conditional on delegation (R6, R8). */
export const ICANN_HINT =
  "Someone applied for this string as a new gTLD. If ICANN delegates it, resolvers that use the ICANN root answer for that TLD, not for this Handshake name.";
