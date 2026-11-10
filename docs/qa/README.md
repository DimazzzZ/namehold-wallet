# Agent-run GUI QA scenarios

A library of manual test cases for the Namehold desktop app, written to be run again and again by a QA agent: an agent driving the real app (Tauri 2, macOS) through computer use or an accessibility driver (e.g. cua-driver), against the local regtest chain (and, for the HNS ⇄ BTC exchange, a local Bitcoin Core regtest node and a local relay). Some cases also run in the browser QA mock (`pnpm dev`) through browser automation. The cases cover what unit and live Rust tests cannot see: real IPC, the secure windows that sign, real node sync, native Save and Open panels, the Shakedex CLI on the other side of a sale, and the other party, the relay and the watchtower of an exchange trade.

The definitions live here; the results of a run never do. The short checklists in [QA_WORKFLOWS.md](../QA_WORKFLOWS.md) stay the overview; these files expand them into steps with exact expected text.

## Files

| File | What it holds |
|---|---|
| [rules.md](rules.md) | the hard rules of every run (regtest only, real data, secrets, the mining budget) |
| [environment.md](environment.md) | build and install, the modes, the regtest node, the CLI seller, profiles, funding, test names |
| [ui.md](ui.md) | where things are in the app, secure windows, listing states, known quirks and known issues |
| [procedures/](procedures/) | reusable step sequences the cases call by name |
| [scenarios/](scenarios/) | the cases, one file per area |
| [report-template.md](report-template.md) | the skeleton of a run's report |

Procedures: [mine-and-sync](procedures/mine-and-sync.md), [switch-profile](procedures/switch-profile.md), [lock-and-list](procedures/lock-and-list.md), [cancel-and-bring-home](procedures/cancel-and-bring-home.md), [restore-and-import](procedures/restore-and-import.md), [cli-list-a-name](procedures/cli-list-a-name.md), [cli-fill](procedures/cli-fill.md), [buy-a-cli-listing](procedures/buy-a-cli-listing.md), [seed-many-names](procedures/seed-many-names.md); for the exchange [exchange-setup](procedures/exchange-setup.md) and [exchange-time-travel](procedures/exchange-time-travel.md).

## Scenario index

| File | Area | Cases | Priorities | Blocks (approx.) |
|---|---|---|---|---|
| [shakedex/selling.md](scenarios/shakedex/selling.md) | lock, Finalize & sign, Listed, Sold, lower price, cancel and home | S1 S2 S3 S4 S5 S6 S7 S9 S10 S11 | P1–P3 | 70 |
| [shakedex/buying.md](scenarios/shakedex/buying.md) | import, verify, buy, finalize, races, hotkeys, one filled button, Speed up of a purchase | B1 … B20 | P1–P3 | 60 |
| [shakedex/restore-and-import.md](scenarios/shakedex/restore-and-import.md) | restore a lock by name, own-file import, second device, a cancel or sale from another device | R1 … R6, R8, R9, C4 | P1–P3 | 103 |
| [shakedex/gates-and-refusals.md](scenarios/shakedex/gates-and-refusals.md) | profile kind, sending, experimental flag, locked signer, input refusals | G1 G2 G3 S8 S12 X4 | P1–P3 | 2 |
| [shakedex/persistence-and-recovery.md](scenarios/shakedex/persistence-and-recovery.md) | restarts, unsent drafts, node outages, background jobs, the daemon's finalize reminders | X1 X2 X3 X5 J1 J2 J3 J4 | P1–P3 | 15 (+22 for J4 on a spare name) |
| [shakedex/activity-and-history.md](scenarios/shakedex/activity-and-history.md) | Activity labels and amounts, consistency, balance, no paid swap entry point | C1 C2 A1 A2 | P1–P3 | 0 |
| [shakedex/profiles.md](scenarios/shakedex/profiles.md) | per-profile reads and signing | R7 X6 | P2–P3 | 0 |
| [shakedex/mainnet-absence.md](scenarios/shakedex/mainnet-absence.md) | mainnet-only parts checked by absence, the mainnet Market page and the listing expiry line in the browser mock; not checkable on regtest | M1 M2 M3 M5 M6 C5 | P1–P3 | 0 |
| [wallet/speed-up.md](scenarios/wallet/speed-up.md) | Speed up (child pays for parent) of an ordinary send: the form, the secure window, refusals, the linked rows | U1 U2 U3 | P1–P2 | 3 |
| [names/batch-and-ownership.md](scenarios/names/batch-and-ownership.md) | batch actions on 200+ names (capabilities, a series of transactions, blockers named and dropped, the fee shown), a name in transfer, a lost auction, the lock banner | N1 … N7 | P1–P3 | ~36 setup + ~45 |
| [exchange/gates-and-settings.md](scenarios/exchange/gates-and-settings.md) | who may trade (watch-only, Ledger, Explorer, no sending), enabling, no relay, the Exchange settings and their secure window, the bitcoind login's secrecy | E1 … E7 | P1–P3 | 0 |
| [exchange/btc-account.md](scenarios/exchange/btc-account.md) | the BTC account: receive address, balance, withdrawal and its secure window, the refusal without a fee estimate | E8 E9 E36 | P1–P3 | 0 (BTC 2) |
| [exchange/offers-and-taking.md](scenarios/exchange/offers-and-taking.md) | sell and buy offers, the order book on the other profile, own offers, the Take dialog and its exact BTC, withdrawing, the per-offer limit and its slots, an offer nobody renews | E10 … E16, E37 | P1–P3 | 0 |
| [exchange/trades.md](scenarios/exchange/trades.md) | the happy path: both Commits and their secure windows, the locks, the reveal and the claim without a click, with the app closed, with either party the maker | E17 E18 E19 | P1–P2 | 6 (BTC 6) |
| [exchange/refunds-and-safety.md](scenarios/exchange/refunds-and-safety.md) | the Seller never commits, the Buyer offline before the reveal, the Seller offline after it (the watchtower claims), the honest-wallet rule, a wrong-amount lock, the reveal deadline, a decoy coin | E20 … E25, E35 | P1–P3 | ~54 (BTC ~80) |
| [exchange/recovery.md](scenarios/exchange/recovery.md) | cancelling, a kill right after a Commit, a deadline passing in the Commit window, relay outages, the watchtower line, restarts, Activity and notifications, the browser mock | E26 … E34 | P1–P3 | ~8 (BTC ~8) |

Case ids are unique across all files. Setup ([environment](environment.md#test-names-for-a-run)) costs about 90 blocks more. The exchange's Bitcoin blocks (in brackets) are mined on its own bitcoind and do not count against the mining budget; its setup ([exchange-setup](procedures/exchange-setup.md)) costs about 12 HNS blocks.

## Picking cases

- **By priority.** P1 is the core of the feature and every fix that a regression would break; run all P1 first, then P2, then P3. A run may stop after any priority level; the report says where it stopped.
- **By area.** For a change in one area, run that file's P1 and P2 cases, plus [C1](scenarios/shakedex/activity-and-history.md) and [A1](scenarios/shakedex/activity-and-history.md) at the end.
- **By regression.** A case that names `Regression of:` exists because of a fixed bug; rerun it whenever the code around that fix changes.
- Respect the mining budget ([rules](rules.md#the-regtest-chain)): add up the cases' `Blocks mined` before you start.

## Suggested run order (saves blocks)

1. Build, install, record ([environment](environment.md#build-and-install-from-the-branch-head)); start the node; record the height. Setup: names and CLI listings, funding (~90 blocks).
2. P1: B1, B2, B9, B7, B8 (11); when the run will reach P2, fold in B18 (P2) between B9 and B7, B20 (P2) inside B7 between its steps 2 and 3 (0), and B19 (P2) inside B8, while their listings are still buyable; B12 (1+); S2 with M1, X1(a)(b) and X4 folded in (22), and, when the run will reach P2 and the maintainer watches the screen, J4 (P2) at its two `mine 9` steps (0); S3 (12); R1 with S1, R4(c) and, at P2, R9(a) folded in (22); R2 (12); U1 (1); G1; X1(c) (2); X2 (≤ 10); then C1 and C2 over everything mined.
3. P2: B3–B6, then B11 before B10 (≤ 23); S4 (12); S5 (23); S6, S7, S12 (2); R3 (22); R4, R5; C4 with R7 inside it (23); R9(b) after C4 (12); U2 (1), U3 (1); G2, G3; J1; J4 on a spare name if S2 ran without it (22); X3, X4, X5; A1; M5 and M6 in the browser mock.
4. P3: B13 (13), B14 (11, or 0 if later cases mine enough), B15, B16, B17; S8, S9, S10, S11; R6; J2, J3; X6; M2, M3; A2; record C5.

About 380 blocks in all, about 570 in the worst case with retries.

## Exchange run order

The exchange cases need [exchange-setup](procedures/exchange-setup.md) (mode B is assumed) and take wall-clock time: on regtest a trade's HNS refund opens 60 minutes after it starts and its BTC refund 120 minutes after. [exchange-time-travel](procedures/exchange-time-travel.md) moves a chain's median time: in real time (always allowed), or by mocktime (the exchange's own bitcoind; on hsd only with the maintainer's agreement, [rules](rules.md#the-regtest-chain)). After a mocktime jump no new trade may start until both chains' median time is back at or before the real clock, so the cases that travel come last.

1. Setup, with [E36](scenarios/exchange/btc-account.md) on a fresh Bitcoin chain before its fee estimator is seeded, [E4](scenarios/exchange/gates-and-settings.md) folded in on the Seller before it is enabled, [E6](scenarios/exchange/gates-and-settings.md) as the first settings save, and [E8](scenarios/exchange/btc-account.md) at the Buyer's funding.
2. E10, E11, then E14 at once: every trade case starts with a take, so a Take dialog that refuses the offers blocks them all.
3. P1 without waiting: E1, E3, E9, E12, E17 with E26(d) folded in (between its steps 4 and 5, and after 8), E18, E26(a)(b), E27(a).
4. P1 that wait, started back to back so one wait serves them all: E20 with [E30](scenarios/exchange/recovery.md) folded in before its Commit, E24, E26(c); then E23 (from its step 4 the daemon is off and the Buyer stays active: start it once the others no longer need a Seller click); then E22 (about 10 minutes, daemon off, Buyer active). E21 last, alone (the app and the daemon stay off until both its refunds).
5. P2: E37; E2, E5, E7, E11's step 2, E13, E15, E19, E27(b)(c), E28, E29, E30 if not folded, E34, E31.
6. P3: E16; E35; E25 alone (no HNS block for 30 minutes); E32 over everything mined; E33 in the browser mock.

With real time only, the waiting group takes about 2½ hours (the BTC refunds) and a full run 4–5 hours; with mocktime on both chains, well under 2 hours. About 60 HNS blocks and 110 Bitcoin blocks in all.

## Verdicts

| Mark | Meaning |
|---|---|
| PASS ✅ | seen as expected: in a snapshot, a screenshot, a node RPC answer or a file check, never from inference |
| PARTIAL ◐ | part seen as expected, part not observable or not reached; say which part and why (a missed transient toast, an unreadable hover, a known spec §5 gap) |
| FAIL ❌ | differs from Expected; give the exact on-screen text, a screenshot and the txid or chain fact |
| BLOCKED ✗ | a precondition or the environment prevents the case (mainnet-only, no device, budget); say what blocked it |
| — not run | skipped by choice (priority level, time); say why |

## Reports

A report is written per run, from [report-template.md](report-template.md), **outside the repo or in a gitignored folder** (for example `.qa/runs/<date>-<tag>/` in the checkout), together with its screenshots and listing files. It is never committed. Its header carries the build's commit SHA, the installed binary's SHA-256, the mode, the GUI driver, and the chain height at the start and at the end; every case row carries its own start and end heights. Findings get run-local ids (Q1, Q2, …); once a finding is fixed and a case added for it, the case's `Regression of:` line names it.

## Adding or changing a case

- Change a case only when the behaviour it checks changes (a new sentence, a new state), or when its wording was wrong against the code; never to make a failing run pass.
- Keep ids stable and **never reuse an id**, also after a case is deleted. A new case takes the next free number in its letter (S13, B17, R8, …).
- When two cases check the same behaviour, keep one and list the other's id in its `Was:` line.
- Every expected UI string is copied from the source (`src/components/**`, `src/components/market/marketText.ts`, the backend's sentences in `src-tauri/src/commands/*.rs` and `src-tauri/src/noncustodial/**/*.rs` and `src-tauri/src/error.rs` (refusal constants, confirmation rows, notifications, error texts), `src/test/fixtures/listing-actions.json`); a string that could not be checked carries `(unverified wording)`.
- No run-specific facts in a case: no heights, txids, real names (use the roles of [environment](environment.md#test-names-for-a-run)), dates or commit references. Update the file's `last_verified` front matter after a run that executed it.
- Use this shape:

```markdown
## <ID> — <title>

- **Priority:** P1 | P2 | P3
- **Was:** <older ids of the same behaviour, with their source>
- **Covers:** <spec R-numbers>
- **Regression of:** <finding id> (<one phrase>)   # only when the case exists because of a fixed bug
- **Preconditions:** <state needed, roles, profile>

Steps:

1. <action, with exact button labels in backticks>

Expected:

- <exact on-screen text, quoted>
- Passes if <the condition>.

- **Blocks mined:** <n>
- **Evidence:** <screenshots, RPC output, files>
- **Cleanup / end state:** <what the case leaves behind>
```
