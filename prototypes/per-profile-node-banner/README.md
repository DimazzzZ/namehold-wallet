# Per-profile node preflight banner — prototype

Throwaway UI prototype for ADR-001 (per-profile node configuration with global fallback).

## What it tests

- The three-state preflight banner (ok / warn / err) surfaced on WalletView.
- The modal "Configure node for this profile…" opened from the banner.
- The resolution order override → global → default, and the interaction with
  N11 `realign_loopback_rpc_url` (realign fires only when there is no override).

## How to run

Open `index.html` in any browser. No build step, no deps.

Use the left panel to change inputs, or click "Cycle preset" to walk through the
five canonical states we want to grill:

1. Happy path — override present, node matches, synced.
2. Positive network mismatch — override points at a mainnet node from a regtest profile.
3. Unreachable override — override endpoint times out.
4. No override, realign fires — global URL is a stale default for a different network.
5. No override, global fine — plain global settings, node syncing.

## What we're grilling

- Is the "Configure node for this profile…" button in the right place, or should
  it live in Settings only?
- Should the banner show the resolved endpoint URL, or is that noise?
- On unreachable override — is "Fall back to global" the right escape hatch, or
  does it violate ADR-001's "explicit choice is never silently abandoned"?
- Is "Node is still syncing" a warning or a soft-ok? Send is currently allowed;
  should it be?
- On mismatch, do we surface "Open Settings → Connections" *in addition* to
  "Configure for this profile", or is that two ways to do the same thing?

