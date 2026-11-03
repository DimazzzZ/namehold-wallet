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

/**
 * The ASCII name an IDN reads as, when every letter of it that is not plain
 * ASCII looks like a Latin one: `.аpple`, in Cyrillic, reads as `apple`. `null`
 * for a plain ASCII name, and for an IDN with any letter that gives it away,
 * such as `сбер`. A name is the buyer's to check, so this only informs.
 */
export function readsAsLatin(name: string): string | null {
  const decoded = displayName(name);
  if (decoded === name) return null;
  let latin = "";
  for (const ch of decoded) {
    const mapped = ch.charCodeAt(0) < 0x80 ? ch : LOOK_ALIKES[ch];
    if (!mapped) return null;
    latin += mapped;
  }
  return latin || null;
}
