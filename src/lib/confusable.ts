import { displayName } from "./idn";

/**
 * Letters of other scripts that render like a Latin letter. Hand-written from
 * Unicode's confusables, lower case only since IDNA folds case: a table of the
 * common ones, not all of them, so no warning does not prove a name is safe.
 * Accented Latin is left out: `münchen` reads as itself, not as `munchen`.
 */
const LOOK_ALIKES: Record<string, string> = {
  // Cyrillic
  а: "a",
  е: "e",
  һ: "h",
  і: "i",
  ј: "j",
  к: "k",
  ӏ: "l",
  о: "o",
  р: "p",
  ԛ: "q",
  ѕ: "s",
  ѵ: "v",
  ԝ: "w",
  х: "x",
  у: "y",
  с: "c",
  ԁ: "d",
  // Greek
  α: "a",
  ι: "i",
  κ: "k",
  ο: "o",
  ρ: "p",
  υ: "u",
  ν: "v",
  χ: "x",
  // Armenian
  ո: "n",
  ս: "u",
  // Latin letters that pass for another
  ı: "i",
  ɑ: "a",
  ɡ: "g",
};

/** Anything a reader cannot see as part of a name: all but letters, marks, digits and `-`. */
const NOT_A_LETTER = /[^\p{L}\p{M}\p{N}-]/u;

/** What a decoded name hides or imitates; see {@link confusable}. */
export interface Confusable {
  /** The plain ASCII name it reads as, when every letter looks Latin. */
  readsAs: string | null;
  /** It holds characters that do not show, such as a zero-width space. */
  hidden: boolean;
}

/**
 * Whether an IDN hides characters or reads as an ASCII name: `.аpple`, in
 * Cyrillic, reads as `apple`. The chain takes any `xn--` label, so a label can
 * decode to what IDNA would refuse: anything that is not visibly a letter and
 * compatibility forms are read away first, or one of them would take a look-alike
 * out of the table.
 * `null` for a plain name, and for an IDN with a letter that gives it away, such
 * as `сбер`. A name is the buyer's to check, so this only informs.
 */
export function confusable(name: string): Confusable | null {
  const decoded = displayName(name);
  if (decoded === name) return null;
  const hidden = NOT_A_LETTER.test(decoded);
  // After NFKC an accent a letter takes is part of it (`ü`); a mark left over
  // sits on a look-alike and must not take it out of the table.
  const visible = decoded
    .replace(new RegExp(NOT_A_LETTER, "gu"), "")
    .normalize("NFKC")
    .replace(/\p{M}/gu, "")
    .toLowerCase();
  let latin = "";
  for (const ch of visible) {
    const mapped = ch.charCodeAt(0) < 0x80 ? ch : LOOK_ALIKES[ch];
    if (!mapped) {
      latin = "";
      break;
    }
    latin += mapped;
  }
  return latin || hidden ? { readsAs: latin || null, hidden } : null;
}
