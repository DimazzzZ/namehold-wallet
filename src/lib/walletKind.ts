import type { WalletProfileSummary } from "../types";

/** How a wallet signs, in one word. */
export function walletKindLabel(
  p: Pick<WalletProfileSummary, "kind" | "watchOnly">,
): "Ledger" | "Watch-only" | "Hot" {
  return p.kind === "ledger_hardware" ? "Ledger" : p.watchOnly ? "Watch-only" : "Hot";
}
