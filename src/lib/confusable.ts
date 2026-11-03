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
 * decode to what IDNA would refuse: invisible characters and compatibility forms
 * are read away first, or one of them would take a look-alike out of the table.
 * `null` for a plain name, and for an IDN with a letter that gives it away, such
 * as `сбер`. A name is the buyer's to check, so this only informs.
 */
export function confusable(name: string): Confusable | null {
  const decoded = displayName(name);
  if (decoded === name) return null;
  const hidden = /[\p{Cf}\p{Default_Ignorable_Code_Point}]/u.test(decoded);
  const visible = decoded
    .replace(/[\p{Cf}\p{Default_Ignorable_Code_Point}]/gu, "")
    .normalize("NFKC")
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
