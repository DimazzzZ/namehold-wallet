/**
 * The local node of a wallet's network (spec 2026-10-24): how its network is
 * named to a person, and where its per-network settings live.
 */

/**
 * The name a person reads for an hsd network: hsd and `node_status` say
 * `main`, a wallet profile says `mainnet`.
 */
export function networkDisplayName(network: string): string {
  return network === "main" ? "mainnet" : network;
}

/**
 * A node control's text for the active wallet's network (R8): "Start mainnet
 * node", or "Start node" with no active wallet, whose network is `null`.
 */
export function nodeActionLabel(
  verb: "Start" | "Stop" | "Starting",
  network: string | null,
): string {
  const node = network ? `${networkDisplayName(network)} node` : "node";
  return verb === "Starting" ? `Starting ${node}…` : `${verb} ${node}`;
}

/**
 * The settings key of a per-network node setting (R10): `node_mode.main` for
 * a mainnet wallet, spelled as the backend's `Network::as_str` spells it.
 */
export function networkNodeSettingKey(
  base: "node_mode" | "autostart_hsd",
  profileNetwork: string,
): string {
  return `${base}.${profileNetwork === "mainnet" ? "main" : profileNetwork}`;
}

/** Why Stop is disabled for a node Namehold did not start (R7); the backend's refusal says the same. */
export const OUTSIDE_NODE_NOT_STOPPED =
  "This node was not started by Namehold, so Namehold does not stop it.";

/**
 * What re-syncing the node does, asked before it runs: Settings › Node and the
 * readiness popover share this one confirm. True when the user agreed.
 */
export function confirmResync(): boolean {
  return window.confirm(
    "Re-sync node data?\n\nYour current chain will be moved to a timestamped " +
      "backup folder in the data directory, and hsd will re-sync from scratch " +
      "with the indexes the wallet needs. This can take a while. Your wallet " +
      "keys are NOT affected.",
  );
}
