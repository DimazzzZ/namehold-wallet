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
