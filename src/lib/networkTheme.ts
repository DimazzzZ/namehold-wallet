import { networkDisplayName } from "./nodeNetwork";

export type NetKey = "mainnet" | "testnet" | "regtest" | "simnet";

export interface NetTheme {
  label: string;
  text: string;
  dot: string;
  card: string;
  badge: string;
  /** Full-width strip at the top of the window; null on mainnet. */
  strip: string | null;
  note: string | null;
}

// Tailwind only generates classes it finds as literal strings: write each one
// out in full. These hues are reserved for networks (DESIGN_SYSTEM §2).
export const NET_THEME: Record<NetKey, NetTheme> = {
  mainnet: {
    label: "Mainnet",
    text: "text-emerald-700",
    dot: "bg-emerald-500",
    card: "bg-gray-50 border-gray-200",
    badge: "bg-emerald-50 text-emerald-700 border-emerald-200",
    strip: null,
    note: null,
  },
  testnet: {
    label: "Testnet",
    text: "text-amber-800",
    dot: "bg-amber-500",
    card: "bg-amber-50 border-amber-300",
    badge: "bg-amber-50 text-amber-800 border-amber-300",
    strip: "bg-amber-400 text-amber-950",
    note: "Test coins — they have no value",
  },
  regtest: {
    label: "Regtest",
    text: "text-purple-800",
    dot: "bg-purple-600",
    card: "bg-purple-50 border-purple-300",
    badge: "bg-purple-50 text-purple-800 border-purple-300",
    strip: "bg-purple-700 text-white",
    note: "Local test chain — blocks are mined on demand",
  },
  simnet: {
    label: "Simnet",
    text: "text-teal-800",
    dot: "bg-teal-500",
    card: "bg-teal-50 border-teal-200",
    badge: "bg-teal-50 text-teal-800 border-teal-200",
    strip: "bg-teal-700 text-white",
    note: "Simulation network",
  },
};

/** The theme of a wallet's network; accepts hsd's "main" too, unknown → neutral gray. */
export function netTheme(network: string | null | undefined): NetTheme {
  const key = network ? networkDisplayName(network) : null;
  if (key && key in NET_THEME) return NET_THEME[key as NetKey];
  return {
    label: network ?? "",
    text: "text-gray-700",
    dot: "bg-gray-400",
    card: "bg-gray-50 border-gray-200",
    badge: "bg-gray-100 text-gray-700 border-gray-200",
    strip: null,
    note: null,
  };
}
