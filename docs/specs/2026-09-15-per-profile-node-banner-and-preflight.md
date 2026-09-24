# Per-profile node banner & preflight — UX/behaviour spec

Status: accepted (verdicts captured from prototype grilling, 2026-09-15).
Scope: how the wallet surfaces per-profile node state to the user, what actions
the user is offered when the effective node config is broken, and how the
preflight cache interacts with broadcasts. This spec is the UX/behaviour layer
on top of [ADR-001 Per-profile node configuration](../adr/0001-per-profile-node-configuration.md);
the ADR defines *what is stored and how it resolves*, this spec defines *what
the user sees and can do*.

A throwaway prototype answered these questions and was deleted with them; the
verdicts below are its durable output, implemented step by step in the real app.

Progress against the plan at the bottom of this file:

| Step | State |
|------|-------|
| 1. Schema + resolver | Done. `profile_settings` (migration 027), `effective_node_config_for_profile`. |
| 2. Migrate call sites | Done. Every node client resolves per profile; realign runs only on fallback resolution; an unresolvable profile config is an error, never a fallback to global. |
| 3. Preflight command | Not started. No `profile_preflight` exists. |
| 4. Banner UI | Not started. |
| 5. Preflight cache | Not started. |
| 6. Override modal | Not started — so nothing writes `profile_settings` outside tests, and the override is not yet reachable by a user. |
| 7. Write-path fresh preflight | Not started. |
| 8. Per-slot read/write overrides | Not started. The table has no slot column, and the resolver returns one tuple. |
| 9. E2E matrix test | Not started. |

## Vocabulary (from CONTEXT.md)

Profile, network, node, chain source, node mode, node configuration,
per-profile override, preflight, network mismatch, effective node config —
used exactly as defined in [CONTEXT.md](../../CONTEXT.md). This spec does not
redefine them.

## Canonical preflight priority

When several conditions are true at once, the FIRST match wins. Keep any
implementation in this exact order; do not reorder without updating this
contract.

1. **Reachability** — can we talk to the node/explorer at all?
2. **Authorization** — reached, but the endpoint rejected our credentials (401/403).
3. **Network mismatch** — reachable, but reports a chain that doesn't match the profile's network (after `mainnet ↔ main` normalization).
4. **Fresh-setup** — new profile inheriting a global config that can't serve it (e.g. profile is regtest, global points at mainnet).
5. **Syncing** — right network, node not caught up yet.
6. **Ready** — reachable, right network, ready.

Rationale: we can't judge a node's network until we can reach it, and a node's
sync state on the wrong network is meaningless.

## Verdicts

Each verdict is stated as `# — decision — rationale`. Numbering matches the
grilling transcript to preserve traceability; gaps are intentional (Q1–Q26
covered decisions already captured in ADR-001 / ADR-002; only Q27+ are new UX
decisions surfaced by the prototype).

### Banner behaviour

- **V-B1 (Q27) — no one-tap "Fall back to global" on an unreachable override.**
  When a per-profile override is unreachable, the error banner offers only
  "Configure node for this profile…". Clearing the override happens inside the
  modal, as an explicit deliberate action. A one-tap escape hatch on the banner
  would repeat exactly the silent-fallback failure mode ADR-001 refuses.

- **V-B2 (Q28) — banner action = "Configure node for this profile…", not "Open Settings".**
  The banner takes the user directly to the per-profile override modal, not
  to the global Settings → Connections page. Global settings remain reachable
  from Settings for users who don't have (or want) an override.

- **V-B3 (Q29) — one primary button on the banner; no secondary "Open Settings" link.**
  Two actions on an error banner turn "read the message, take the action" into
  "read the message, choose between two actions". Keep it one primary CTA.

- **V-B4 (Q30) — banner shows the resolved *host:port* only, not the full URL.**
  Credentials, path, and query never appear in the UI. Implementation must
  strip `user:pass@` and everything after the host+port before rendering.

### Override modal

- **V-M1 (Q31) — modal validates URL scheme + host on input, not only on save.**
  Reject non-http(s) URLs and empty hosts inline. Save is disabled while the
  form is invalid. This mirrors the onboarding "Test connection" contract: no
  invalid config is ever committed.

- **V-M2 (Q32) — modal shows live preview: two mini-banners (Current / After save).**
  "Current" preflights the *saved* state; "After save" preflights the *draft*
  (current form values with `hasOverride: true`). The outer banner does NOT
  change until Save is clicked. Node-reality (reach, chain, synced) for the
  draft comes from a real probe, not from the saved cache — draft URL is by
  definition different from what's cached.

- **V-M3 (Q33) — "Clear override" is a distinct button in the modal, not a URL trick.**
  Emptying the URL field is a validation error, not a shortcut for clearing.
  Clearing is an explicit action with its own affordance so users don't
  accidentally wipe an override by fumbling the URL.

### Cache & action semantics

- **V-C1 (Q34) — preflight cache is for *rendering only*.**
  Broadcasts and syncs ALWAYS run a fresh preflight and IGNORE the cache. The
  banner is an indicator, never the source of truth for a write. This makes
  the write path TTL-independent by construction.

- **V-C2 (Q35) — cache TTL for rendering = 30s; lazy invalidation on effKey drift.**
  Cache entries are keyed by `(profile_id)` and validated by an `effKey` of
  the resolved effective config (`url|chain_source|source`). Two defenses:
  (a) TTL expiry after 30s treats the entry as a miss; (b) an effKey mismatch
  (global setting changed, override added/cleared) also treats it as a miss.
  Profiles whose effective config is unchanged keep their cache across switches.

- **V-C3 (Q36) — write failure preflight shows "…(preflight ran just now)".**
  When a fresh preflight refuses the write, the user-visible error names the
  refusal reason AND states the preflight was fresh, so the user knows this
  wasn't a stale-cache decision.

### Network mismatch & syncing

- **V-N1 — positive network mismatch is a hard error, not a warning.**
  If the reached node reports a chain that after normalization is not the
  profile's network, reads and writes are refused. The banner CTA is
  "Configure node for this profile…" when the source is a profile override,
  and "Open Settings" when the source is the global fallback.

- **V-N2 — "Unknown chain" (node didn't report) is a soft warning, not a block.**
  Reads are allowed; writes are gated by the same logic as reads (i.e. only
  the positive mismatch blocks writes). Consistent with ADR-002 principle:
  don't punish the user for ambiguous evidence.

- **V-N3 — "Syncing" is a soft warning; sends are allowed but flagged.**
  A syncing node can still broadcast correctly; blocking sends would be
  overreach. The banner surfaces the sync gap so the user isn't surprised
  when their tx appears delayed.

### Realign interaction (already in ADR-001, restated for completeness)

- **V-R1 — realign fires only when the resolved source is the global fallback.**
  When the profile has an override (either read or write slot), realign is
  skipped for that slot. Silently rewriting an override — even a stale
  loopback default — would repeat the exact failure mode ADR-002 refuses.

### Per-slot overrides (read vs write)

- **V-S1 — each profile has independent `read` and `write` slots.**
  A profile may read via explorer while writing via its own node, or override
  only one slot and inherit the other from global. Resolution and preflight
  run per slot; the banner reports the worst of the two.

- **V-S2 — global write is refused when `allow_remote_broadcast != "true"`.**
  A remote-node write source is disabled unless the user explicitly opts in,
  consistent with R11 of the remote-node spec.

## Non-goals

- **Not a health monitor.** The banner does not poll continuously; it refreshes
  on profile switch, on override save, on settings change, and lazily on
  render when the cache is stale/expired.
- **Not an onboarding replacement.** The onboarding "Test connection" flow
  remains the way a fresh install validates its first node. The banner takes
  over after onboarding completes.
- **Not multi-node failover.** A profile has exactly one effective read source
  and one effective write source at a time.

## Test expectations

A canonical acceptance matrix covers these state combinations for a profile
(all with a valid fresh preflight):

| # | override | reach       | chain match | synced | banner level | writes |
|---|----------|-------------|-------------|--------|--------------|--------|
| 1 | none     | ok          | yes         | yes    | ok           | yes    |
| 2 | override | ok          | yes         | yes    | ok           | yes    |
| 3 | override | unreachable | —           | —      | err          | no     |
| 4 | override | ok          | no          | —      | err          | no     |
| 5 | none     | ok          | no          | —      | err          | no     |
| 6 | any      | ok          | yes         | no     | warn         | yes    |
| 7 | any      | ok          | unknown     | —      | warn         | yes    |
| 8 | override | unauthorized| —           | —      | err          | no     |

Row #3 must offer only the "Configure node for this profile…" CTA (V-B1). Row
#5 must offer "Open Settings" (V-N1 second half). Row #6 must not block send.
Row #8 is Q15 (authorization); same shape as reachability.

## Implementation plan (step by step)

Ordered so each step is independently shippable and testable. Each step is a
PR-sized unit.

1. **Schema + resolver (backend).** Add `profile_settings` table (per ADR-001),
   implement `effective_node_config_for_profile(profile_id)` in Rust with
   unit tests for the resolution order (override → global → default), realign
   skip when override present, and the network-normalization helper. No
   frontend changes yet; existing global-only call sites continue to work.

2. **Migrate call sites to the resolver.** Route every site that currently
   reads `node_rpc_url` / `node_rpc_api_key` / `chain_source` from global
   settings through the new resolver. One PR per subsystem (sync, read, tx,
   chain_scan, daemon) so regressions are bisectable. Realign now runs only on
   fallback resolution.

3. **Preflight command.** Add a Tauri command `profile_preflight(profile_id)`
   that returns `{ effective, reach, chain, synced, level, title, body,
   actions, source, host_port }`. Backed by the same probe used by
   `check_node_connection` but resolved against the profile. Unit-tested with
   fixtures for each of the 8 canonical rows above.

4. **Banner UI in Layout.** Replace the current sidebar "READ-ONLY / CAN SEND"
   chip with a stateful banner slot in `src/components/Layout.tsx`, driven by
   a new hook `useProfilePreflight()`. Renders host:port only (V-B4); primary
   CTA per V-B2/V-B3.

5. **Preflight cache (frontend).** In-memory `Map<profile_id, entry>`
   invalidated by (a) 30 s TTL, (b) effKey drift, (c) explicit "save
   override", (d) profile switch. Rendering path only (V-C1).

6. **Override modal.** New `ProfileNodeOverrideModal` component with URL/CS
   inputs, inline validation (V-M1), live-preview mini-banners (V-M2),
   distinct "Clear override" button (V-M3). Wired from the banner CTA and
   from Settings.

7. **Write-path fresh preflight.** Add a fresh-preflight step to every write
   entry point (send, batch send, name actions, register, finalize, transfer).
   Refuse the write if the fresh preflight is not `ok`, surfacing V-C3's
   "preflight ran just now" wording.

8. **Per-slot read/write overrides.** Extend the resolver + modal to support
   independent read and write overrides (V-S1) and enforce
   `allow_remote_broadcast` on the write slot (V-S2). This can slip to a
   later PR if slot-splitting is out of scope for the first ship.

9. **E2E matrix test.** Playwright test that drives the 8 canonical rows via
   the webqa mock (`src/lib/webqa-mock.ts`) — profile switch, banner text,
   CTA target, write allowed/refused.

Verification cadence per step: `npx tsc -b --noEmit`, `npx vitest run` for
the touched components/hooks, and `cargo test` for the touched Rust crate.
Final step also runs the Playwright suite.
