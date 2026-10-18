---
area: Shakedex across regtest profiles of one recovery phrase
priority: P2–P3
blocks: 0
requires:
  - environment.md (seller and second-device profiles)
  - procedures/switch-profile.md
  - rows that differ between the two profiles (C4 step 5, or R1/R2 rows)
last_verified: "eec6b2d (same tree as 150d58a) for R7; bc8cd4d for X6; 2026-10-18, report kept outside the repo"
---

# Profiles

Every wallet-scoped read is for the profile it names. Switching profiles must never show, or sign for, the other profile's listings. Profile switching itself is in [switch-profile](../../procedures/switch-profile.md).

## R7 — Switching profiles never shows another profile's listings

- **Priority:** P2
- **Was:** full QA R7; T7 fixes recheck C3
- **Covers:** R19, R32 (per-profile listing state)
- **Regression of:** Q2 (after a quick profile switch My listings showed the previous profile's rows under the new profile's header)
- **Preconditions:** a row in one profile that the other does not have, or has in another state (for example SE `Listed for sale` in the seller and `Locked — listing details unknown` in the second device, inside C4 between the restore and the import); run it a second time on rows that ended alike.

Steps:

1. Wallet page → switch profile in the header → go straight to Market › My listings without waiting; snapshot the rows right after the page renders, and again after 5 s.
2. Open Market › Browse right after the switch.
3. Three full cycles seller ↔ second device. Read Owned Names and the balance after each switch too.

Expected:

- My listings never shows a row, state or button of the other profile, not even for a moment; while the new profile's list loads it may show a loading state or nothing, never the old rows.
- Browse in a regtest profile shows "There is no LearnHNS Market for this network. Import a listing file or paste one below." with the import box, and no rows carried over from the other profile.
- Owned Names and the balances never swap.
- The signer-state quirk after a switch is a known issue; don't report it.
- Passes if none of the snapshots (two per switch, six switches) shows a foreign row.

- **Blocks mined:** 0
- **Evidence:** every snapshot of My listings with its time and profile header; any foreign row is ❌ with its snapshot.
- **Cleanup / end state:** back on the seller profile.

## X6 — Switching profile while a secure window is open

- **Priority:** P3
- **Was:** full QA X6
- **Covers:** R16 (signing is for the profile that built the draft)
- **Preconditions:** the seller profile unlocked; any signing action at hand (for example a purchase's `Finalize`).

Steps:

1. Open the action's secure window; do not confirm.
2. In the main window switch to the second device.
3. Return to the secure window and confirm (or decline); switch back; read Activity, the row and `getrawmempool`.

Expected:

- Either the main window blocks the switch until the prompt is answered, or after the switch the prompt is refused or signs only for its own profile ("the unlocked signer is for a different wallet profile" is the known signer quirk).
- Nothing is signed or sent for the wrong profile; record exactly what happened.
- Passes if no transaction leaves for the profile that did not build it.

- **Blocks mined:** 0
- **Evidence:** `getrawmempool` and the row's state after the attempt.
- **Cleanup / end state:** the seller profile active; nothing pending.
