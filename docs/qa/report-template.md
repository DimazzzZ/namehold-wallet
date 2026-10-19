# QA run report template

Copy the skeleton below into the run's own folder, outside the repo or under the gitignored `.qa/` ([README](README.md#reports)), and fill it in. Never commit a filled report. No recovery phrase, no passphrase, no API key but `test`, no menu-bar contents, nothing outside the app's windows ([rules](rules.md#secrets-and-evidence)). Prose paragraphs stay on one line each.

```markdown
# <area> GUI QA — <date range>

## Build

| | |
|---|---|
| Branch / commit | <branch> / <git rev-parse --short HEAD> |
| Built how | working tree (clean) / detached worktree |
| Installed binary SHA-256 | <sha256 of /Applications/Namehold.app/Contents/MacOS/namehold-wallet> |
| Build output SHA-256 | <sha256 of $CARGO_TARGET_DIR/release/namehold-wallet> (must match) |
| Binary mtime / About version | <stat -f '%Sm'> / <About › Version> |
| Backup of the previous app | /Applications/Namehold.app.qa-bak-<yyyymmdd> |
| Mode / GUI driver | A installed app / B isolated dev build / C browser mock; computer use / accessibility driver |

## Environment

| | |
|---|---|
| Chain height | start <h> → end <h> |
| Profiles used | seller = <name>, second device = <name>, watch-only = <name> |
| Settings changed | <setting: original → changed → restored> |

| Role | Real name | How acquired |
|---|---|---|
| SA | <qfa…> | hsd wallet auction, transferred to the seller |
| X1 | <qxa…> | hsd wallet auction, listed by the CLI (`fixed`, 5 HNS) |

Listing files kept: <paths in the run folder>.

## Results

Stopped after: <P1 / P2 / P3>, because <reason>.

| Case | Priority | Verdict | Heights | Evidence |
|---|---|---|---|---|
| B1 | P1 | ✅ / ◐ / ❌ / ✗ / — | <start> → <end> | <exact on-screen text, screenshot names, RPC fact, file check; for ◐ which part and why> |

## Findings

Only new ones, not the known issues of docs/qa/ui.md.

### Q1 — <severity: blocker / bug / cosmetic> — <one line>

- Repro from a known state: <numbered steps>
- Expected: "<quoted>"
- Seen: "<quoted>"
- Evidence: <screenshot>, <txid>, <info / getrawmempool output>
- Regression of an earlier ✅? <case id, or no>

## Final state

- Names: <role → listed / cancelled / sold / awaiting finalize / owned, in which profile>
- Settings: <each changed setting and its restored value>
- App: <running or stopped>, active profile <name>, signer <locked / unlocked>; node <running>, mempool <empty?>
- Screenshots: <list, in the run folder>
```
