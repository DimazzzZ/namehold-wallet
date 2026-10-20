# Agent-run GUI QA scenarios

A library of manual test cases for the Namehold desktop app, written to be run again and again by a QA agent: an agent driving the real app (Tauri 2, macOS) through computer use or an accessibility driver (e.g. cua-driver), against the local regtest chain. Some cases also run in the browser QA mock (`pnpm dev`) through browser automation. The cases cover what unit and live Rust tests cannot see: real IPC, the secure windows that sign, real node sync, native Save and Open panels, and the Shakedex CLI on the other side of a sale.

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

Procedures: [mine-and-sync](procedures/mine-and-sync.md), [switch-profile](procedures/switch-profile.md), [lock-and-list](procedures/lock-and-list.md), [cancel-and-bring-home](procedures/cancel-and-bring-home.md), [restore-and-import](procedures/restore-and-import.md), [cli-list-a-name](procedures/cli-list-a-name.md), [cli-fill](procedures/cli-fill.md), [buy-a-cli-listing](procedures/buy-a-cli-listing.md).

## Scenario index

| File | Area | Cases | Priorities | Blocks (approx.) |
|---|---|---|---|---|
| [shakedex/selling.md](scenarios/shakedex/selling.md) | lock, Finalize & sign, Listed, Sold, lower price, cancel and home | S1 S2 S3 S4 S5 S6 S7 S9 S10 S11 | P1–P3 | 70 |
| [shakedex/buying.md](scenarios/shakedex/buying.md) | import, verify, buy, finalize, races | B1 … B17 | P1–P3 | 60 |
| [shakedex/restore-and-import.md](scenarios/shakedex/restore-and-import.md) | restore a lock by name, own-file import, second device | R1 … R6, C4 | P1–P3 | 80 |
| [shakedex/gates-and-refusals.md](scenarios/shakedex/gates-and-refusals.md) | profile kind, sending, experimental flag, locked signer, input refusals | G1 G2 G3 S8 S12 X4 | P1–P3 | 2 |
| [shakedex/persistence-and-recovery.md](scenarios/shakedex/persistence-and-recovery.md) | restarts, unsent drafts, node outages, background jobs | X1 X2 X3 X5 J1 J2 J3 | P1–P3 | 15 |
| [shakedex/activity-and-history.md](scenarios/shakedex/activity-and-history.md) | Activity labels and amounts, consistency, balance, no paid swap entry point | C1 C2 A1 A2 | P1–P3 | 0 |
| [shakedex/profiles.md](scenarios/shakedex/profiles.md) | per-profile reads and signing | R7 X6 | P2–P3 | 0 |
| [shakedex/mainnet-absence.md](scenarios/shakedex/mainnet-absence.md) | mainnet-only parts checked by absence; not checkable on regtest | M1 M2 M3 C5 | P1–P3 | 0 |

Case ids are unique across all files. Setup ([environment](environment.md#test-names-for-a-run)) costs about 90 blocks more.

## Picking cases

- **By priority.** P1 is the core of the feature and every fix that a regression would break; run all P1 first, then P2, then P3. A run may stop after any priority level; the report says where it stopped.
- **By area.** For a change in one area, run that file's P1 and P2 cases, plus [C1](scenarios/shakedex/activity-and-history.md) and [A1](scenarios/shakedex/activity-and-history.md) at the end.
- **By regression.** A case that names `Regression of:` exists because of a fixed bug; rerun it whenever the code around that fix changes.
- Respect the mining budget ([rules](rules.md#the-regtest-chain)): add up the cases' `Blocks mined` before you start.

## Suggested run order (saves blocks)

1. Build, install, record ([environment](environment.md#build-and-install-from-the-branch-head)); start the node; record the height. Setup: names and CLI listings, funding (~90 blocks).
2. P1: B1, B2, B9, B7, B8 (11); B12 (1+); S2 with M1, X1(a)(b) and X4 folded in (22); S3 (12); R1 with S1 and R4(c) folded in (22); R2 (12); G1; X1(c) (2); X2 (≤ 10); then C1 and C2 over everything mined.
3. P2: B3–B6, then B11 before B10 (≤ 23); S4 (12); S5 (23); S6, S7, S12 (2); R3 (22); R4, R5; C4 with R7 inside it (22); G2, G3; J1; X3, X4, X5; A1.
4. P3: B13 (13), B14 (11, or 0 if later cases mine enough), B15, B16, B17; S8, S9, S10, S11; R6; J2, J3; X6; M2, M3; A2; record C5.

About 365 blocks in all, about 550 in the worst case with retries.

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
- Every expected UI string is copied from the source (`src/components/**`, `src/components/market/marketText.ts`, the refusal constants in `src-tauri/src/commands/shakedex.rs`, `src/test/fixtures/listing-actions.json`); a string that could not be checked carries `(unverified wording)`.
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
