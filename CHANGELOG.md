# Changelog

## Releases

- [0.5.0](#050---2026-08-20)
- [0.4.1](#041---2026-08-11)
- [0.4.0](#040---2026-08-07)
- [0.3.0](#030---2026-07-29)
- [0.2.1](#021---2026-07-25)
- [0.2.0](#020)
- [0.1.0](#010---2026-06-30)

## [Unreleased]

### Added
- **Several bids on one name.** The wallet allowed a single bid per name per wallet; it now allows as many as you like, each with its own value and lockup, the way Namebase does. Every bid rotates to a fresh receive address, so each gets its own nonce, blind, commitment row and BID coin, and the send-side guards that refused a second one are gone. The name modal's header reads "Latest bid … · lockup … · N of yours", and the bids panel keeps every bid in one list with your own rows tinted, so which are yours is visible without counting.
- **Remote-node onboarding** — the first-run flow now opens with a "How do you want to connect?" step offering three choices: **Local full node** (default; start hsd on this device), **Remote node** (point at an existing hsd RPC, with a "Test connection" button that probes the node before you commit), and **SPV** (lightweight headers-only, read-only). Choosing a source persists `chain_source` + `node_mode` (plus `node_rpc_url` / API key for Remote node) up front so a new user reaches a working read+send wallet without waiting for a full local sync. Your recovery phrase never leaves the device — remote/SPV is a privacy/trust tradeoff, not custody. New Tauri command `check_node_connection` validates a candidate RPC (reachable / height / synced, and — once a wallet profile exists — whether the node's network matches the wallet's; a cross-network node, e.g. testnet-for-mainnet, is flagged with an amber warning under the "Connected" line and is not treated as a usable node) without persisting anything, honoring the existing plaintext-key / non-loopback transport guard. Settings' Chain source selector now offers the same four modes (local full / SPV / remote / explorer) and replaces the separate Node mode dropdown; Settings also gained the same "Test connection" affordance and an "Allow sending via remote node" toggle (`allow_remote_broadcast`, off by default) — the toggle appears both on the onboarding Remote step and in Settings.

### Fixed
- **Update, Transfer, Renew and Revoke are no longer offered on a name you have not won yet.** While an auction is in its reveal phase the node already reports whoever holds the highest reveal as the name's owner, so a wallet leading its own auction looked like an owner and the modal enabled every ownership action. None of them can work: until REGISTER the owner coin is a REVEAL, and Handshake allows a REVEAL to become only a REGISTER or a REDEEM, so each would have been refused by the node. They now require the name to actually be registered, and say "the name is not registered yet" when it is not.
- **An Owned Names row is no longer clickable as a whole.** The row carried its own click handler on top of the buttons in its cells, so pressing a block height ran the cell's handler and then the row's as the click bubbled up — two dialogs stacked on each other, and ticking the select box opened the name dialog. The row's actions are its own controls: the name, the two block heights, and Manage. Keyboard selection is unchanged. The shared `DataTable` still offers an opt-in row click and now ignores clicks that came from a control handling them itself.
- **Each bid now shows the amount its own reveal disclosed.** Existing wallets re-scan once on upgrade (migration 031), because values recorded under the old rule may be sitting on the wrong bid and the scanner never revisits a block it has passed. When one transaction reveals several bids — which is what revealing a name you bid on more than once does — the bid list attached the wrong figure to each row: a 1 HNS bid could be shown as having revealed 3 HNS. The chain scanner paired each reveal with "the earliest bid not yet matched", a heuristic that is indistinguishable from the truth while a wallet holds one bid per name. Handshake actually pairs a name covenant with the coin spent at the same index, so a reveal names exactly one bid; the scanner now keys on that outpoint and there is no guessing left.
- **Redeeming now reclaims every losing bid you placed on a name, and the wallet points you at them.** The sibling of the reveal fault, on the stage after it. `build_redeem_draft` resolved one commitment and looked for its reveal coin at that commitment's address, so a wallet that had outbid itself reclaimed one lockup and could not reach the rest — each bid lands on its own rotated address. Worse, the wallet would not even offer it: `can_redeem` required that you did NOT own the name, but winning with one of your own bids and losing with the others is the ordinary outcome of bidding more than once, and it disqualified exactly that case. And once registered, the guided panel reported "no urgent action" while those lockups sat there. All losing reveals for a name are now redeemed in one transaction, owning the name no longer disqualifies the redeem (holding no reveal besides the winner does), and a registered name with losing reveals left says so. Coin lookups moved from "the newest commitment's address" to the name hash across the wallet — the newest was not even well defined, since `created_at` has second resolution and bids placed in the same second ordered arbitrarily.
- **A lockup stranded in an auction that lapsed is now named, with its amount.** Scoping the bids panel to the current auction (above) made an older bid's money simply vanish from the screen — worse than the wrong number it replaced, because nothing explained where it went. A BID output can only be spent by its own REVEAL, and only while that auction's reveal window is open, so once the window closes the lockup is gone for good. The wallet says so, with the figure, instead of leaving a hole in the balance.
- **A bid you have just placed appears while it waits for a block.** The bids panel listed what the chain scanner had indexed, and a bid sitting in the mempool is not indexed yet — so placing a second bid looked like it had failed. Your own unmined bids are now listed and counted separately, so the panel matches what you just did.
- **Revealing now covers every bid you placed on a name, not just the last one.** The wallet lets you place several independent bids on one name, but Reveal only ever revealed the most recent: it looked up a single commitment (the newest) and built one REVEAL for it. Every other bid was left behind, and a BID output can only ever be spent by its own REVEAL, only while that auction's reveal window is open — so the rest of your lockup became unreclaimable the moment the window closed. A second Reveal did not help either; it failed with "no unspent bid coin", because the lookup was pinned to the address of the bid it had already revealed. All of a name's bids in the current auction now reveal together in one transaction. Worse than the loss was that it was silent: the reveal txid was stamped onto every commitment for the name rather than onto the one actually revealed, so the reveal-deadline warning went quiet for exactly the bids still at risk. It is now stamped per bid. The batch path had the mirror fault — it never stamped at all, so it kept warning about bids it had already revealed — and now stamps each one.
- **The bids panel no longer shows "No bids yet" for a name that has bids.** Two independent faults, either of which emptied it. The chain scanner parsed the block shape of hsd's REST API (`outputs`, a bare address string, amounts in doos) while `getblock` goes through JSON-RPC and returns bitcoind's shape (`vout`, an address object, amounts in HNS), so no block ever parsed: the scanner walked entire chains and indexed nothing while its cursor advanced to the tip. It also used the witness txid rather than the txid, so a bid could never have been matched to the wallet's own. Separately, that cursor was a single global row and the index was keyed by nameHash alone — switching from a mainnet profile to a regtest one left a mainnet height in the cursor, which read as "already caught up" against the regtest tip, and `read_name_bids` took the same stale height as proof of coverage and served the empty index as authoritative instead of falling back to the explorer. Both tables are now keyed by network, and migration 028 resets them so the scanner refills from the chain.
- **A bid from a name's previous auction no longer counts as one of your bids on the current one.** The same fault as the bids index, one table over: `bid_commitments` was keyed by name, so a commitment left over from an auction that lapsed kept counting. The name modal's header read "Latest bid … · 2 of yours" while the bids panel directly below it, which does scope by auction, said "yours: 1" — and the bid and lockup figures in that header came from whichever commitment was newest, regardless of which auction it belonged to. Commitments now record the auction they were placed in (migration 030), backfilled for existing rows from the reveal deadline they already stored. A commitment recovered from the chain has no known auction and still counts: showing a bid that may be dead is a wrong number, hiding a live one loses the lockup.
- **Bids from a name's previous auction no longer appear in its current one.** A name can be auctioned repeatedly — an auction nobody reveals in lapses and the name becomes available again — but the index was keyed by the name, not the auction, so every bid ever seen piled up under one key. A name sitting at "Waiting for Bidding" with no auction open still listed the bids from its last one. Rows now carry the OPEN height the BID covenant names (migration 029), and a reveal can no longer attach itself to an unrevealed bid left over from an earlier auction.
- **A lapsed auction leaves no state behind.** When the node reports a name with no auction at all, the cached row kept every column from the last one — the OPEN height it was opened at, and an owner. `read_name_bids` reads that height to decide which auction's bids to show, so a name displaying "Waiting for Bidding" with nothing open still listed the bids of the dead one. Every column that comes from the node's answer is now cleared with it.
- **One modal for a name, not two.** `NameInfoModal` and `NameActionsModal` showed overlapping views of the same name and were reachable from different places. Activity and Auctions now open the single `NameActionsModal`, which carries the phase badge in its title and a read-only details panel where the separate modal used to be.
- **Your own bid is shown before the reveal, instead of zero.** hsd reports a name's on-chain value and highest bid as 0 until amounts are revealed, and the modal printed that. It now shows the wallet's own bid and lockup from its local commitment, compactly, with the exact figure on hover.
- **The bid form no longer appears twice, or where it cannot be used.** The advanced section repeated the bid inputs that the guided panel already showed, and kept offering them outside the bidding window. The inputs appear once, and only where a bid is a thing you can place.
- **Activity shows what a name action actually moved.** The Amount cell reported the transaction's own value for covenant actions, which is the name's locked value rather than what the action cost or returned. It now reads the covenant's name value.
- **A syncing node no longer looks stuck at 99.9%.** Sync was judged by `verificationprogress`, which plateaus just short of 1.0 and sits there, so Settings kept saying "Syncing" and the read paths kept refusing a node that was in fact caught up. The chain tip decides it now — `blocks >= headers` — and progress is only used to describe how far along an actually-unfinished sync is.
- **A profile can no longer adopt or start a node on the wrong chain.** Starting hsd, or adopting one already running, checked nothing about which network it served, so a regtest wallet could attach to a mainnet node and take its height as its own. Both paths now compare the node's chain against the profile's and refuse a definite mismatch, and the data directory is resolved per network so two chains cannot share one.
- **A name whose auction lapsed can be opened again.** The OPEN output is a zero-value marker that nothing ever spends, so the wallet holds it for good — and both the Open button and the server-side guard read "we hold one" as "one is pending". Every name this wallet had ever opened was therefore permanently un-openable, reported as "an auction is already opening for this name (pending confirmation)" even for a name the chain had returned to available. Only an unconfirmed OPEN now counts as pending.
- **Activity shows the name for every name action.** Only BID's covenant had its raw name decoded, so a confirmed OPEN listed its action and then "—" where the name belongs. OPEN and FINALIZE carry the name too and are now decoded; for the covenants that carry no name at all (REVEAL, REDEEM, REGISTER, UPDATE, RENEW), the row falls back to the name recorded in this wallet's own draft.
- **Converting hover hints to the tooltip did not disturb any layout.** The tooltip needs a wrapper element around whatever it describes, and that wrapper arrived with its own `inline-flex`, which turned the sidebar's block-level links into a wrapping grid. A wrapper now takes the layout it is given rather than imposing one, and renders even when it has nothing to say, so a hint appearing or disappearing cannot remount the control underneath it.
- **A disabled action now says why it is disabled.** Every action button carried its reason in a native `title`, and none of them could ever be seen: a disabled button has `pointer-events: none`, so the browser never renders one. Expanding "Show all actions" was a wall of dead buttons with the explanations — a locked signer, the wrong auction phase, a name this wallet does not control — all present and all unreachable.
- **The locked-wallet notice carries an Unlock button, once.** The name-actions modal printed "Unlock your wallet to sign transactions." twice: the modal-wide gate stated it with a button, and the guided panel repeated it underneath as bare text with no way to act on it.
- **The auction description states this network's real periods.** "The name enters a ~1-week bidding period" was a fixed string, but bidding is 720 blocks on mainnet, 144 on testnet, 25 on simnet and 5 on regtest — wrong on every network but mainnet, where it is still loose. The guide now explains the mechanism that is true everywhere and shows the actual block counts beside it.
- **A broadcast OPEN says it is waiting to be mined.** Until it lands in a block the chain still reports the name as available, so the panel rendered "Start a Vickrey auction for this name" with an Open button disabled as "an auction is already opening" — underneath a heading that already read "Wait for Bidding". It invited the user to do the thing they had just done, then explained that they could not.
- **"Start hsd" with no wallet selected now says so instead of starting a mainnet node.** Resolving the network fell back to mainnet on any failure, including having no wallet at all, so the wallet could quietly begin a full mainnet chain sync in a data directory prepared for something else.
- **Test networks no longer claim every name is about to expire.** The "expiring soon" warning was a flat 30 days, but a name's whole lease is about 30 days on testnet and 17 on simnet, so every owned name sat permanently in the alarm state. The threshold now scales with the network's renewal window: mainnet keeps exactly its 30 days, and the other chains get the same share of their own (much shorter) lease.
- **Renewal countdowns stop inventing blocks on regtest.** When no synced node is available the wallet estimates the chain height by ageing its last known one at ten minutes per block. That holds on mainnet and testnet, but regtest and simnet only produce a block when you ask for one, so an idle hour added six blocks that never existed and every countdown drifted. On those chains the stored height is now reported as-is: stale, but true.
- **"View on explorer" no longer offers a dead link off mainnet.** Two of the six explorer links (the name modals) were ungated. Shakeshift indexes mainnet only, so they 404 anywhere else; they now follow the same rule the other four already did.
- **Starting a local node on testnet or regtest now actually connects.** hsd listens on a different RPC port per network (12037 mainnet, 13037 testnet, 14037 regtest), and "Start hsd" already launched it with the right network flag — but the wallet kept talking to the seeded mainnet port, so the node it had just started was unreachable. Starting a node now realigns the stored URL to the network's port. Only a leftover default is rewritten: a custom port or a remote host is left exactly as you set it. The Settings placeholder follows your wallet's network too.
- **Freshly mined coins no longer show up as spendable.** A coinbase output can't be spent until it matures (100 blocks on mainnet and testnet, 2 on regtest, 6 on simnet). Coin selection already knew that, but the balance card didn't: it showed the full amount under "spendable", the Send and Max buttons offered it, and the transaction then failed to build with a bare "Insufficient HNS balance" against a balance the user could see. Immature value now has its own line in the Balance card with the number of blocks left, the spendable figure excludes it, and the send form no longer offers it. If a send does fall short only because of maturity, the error says so instead of claiming the balance is too low.
- **A node on the wrong chain is now refused everywhere, not just on reads.** Pointing the wallet at a node whose network disagrees with the profile (a regtest node for a mainnet wallet, say) used to be caught only by the read gate. The background sync, the chain scanner and the watched-name daemon all consulted a variant of that gate that skipped the comparison, so a foreign chain's height could land in the wallet's sync cursor — the same cursor coin selection reads to decide coinbase maturity — and its name states could overwrite the local cache. Syncing, sending and write-capability are now all gated on the same comparison: `sync_wallet_state` refuses to seed the cache, `get_write_capability` reports the node as unable to send and names both networks, and `broadcast_tx_draft` refuses before the signed transaction leaves the device, leaving the draft untouched. Settings will not save a node the connection test positively identified as mismatched. Only a definite mismatch refuses: a node that doesn't report its chain, or a probe that fails, behaves as before. This reverses the earlier decision to leave cross-chain sends to the node to reject.
- **REGISTER / RENEW / FINALIZE are no longer rejected by the node.** The renewal-block commitment in those covenants was built from a byte-reversed block hash. Unlike Bitcoin, hsd does not reverse hashes for RPC display — `getblockhash` already returns the internal byte order the consensus check reads — so the covenant pointed at an unknown block and the node refused the transaction with `bad-register-renewal`. The hash is now used as returned.
- **Coin selection no longer picks immature coinbase outputs.** Spending a coinbase younger than the network's `coinbaseMaturity` is a consensus violation (`bad-txns-premature-spend-of-coinbase`), which made sends and name actions fail at broadcast on a freshly mined wallet. Coinbase coins are now excluded until they mature, using the network's own threshold (mainnet/testnet 100 blocks, regtest 2, simnet 6) against the wallet's last synced height. A wallet that has never synced treats every coinbase coin as immature until it catches up.
- **SPV mode can no longer broadcast, even past the UI gate.** `ChainSource::SpvNode` is now refused at the RPC broadcast boundary (`can_broadcast()` returns `false`) and up-front in `broadcast_tx_draft`, not only by the UI write-capability gate. A signed draft attempted against an SPV source fails cleanly with a read-only error and is left untouched (neither `failed` nor `broadcast_pending`) — nothing is sent over the wire. Defense-in-depth for the documented "SPV = read-only" product intent.
- **`allow_remote_broadcast` is now enforced at the broadcast boundary, closing a code path that could skip the UI's write-capability gate.** `broadcast_tx_draft` refuses a `remote_node` chain source unless `allow_remote_broadcast` is `"true"`, leaving the signed draft untouched. Previously only the write-capability gate in the UI consulted the flag.
- **The explorer read fallback is now network-gated — no more silent mainnet queries off mainnet.** With no `explorer_api_url` configured, the explorer client factory used to fall back to the mainnet `e.hnsfans.com` index on every network, so a testnet/regtest/simnet wallet whose local node was unreachable would quietly query a mainnet index and report its "not found" as "nothing there". The factory (`explorer_client_from_settings`) now takes the wallet's network and returns `None` off mainnet when no explicit explorer URL is set (resolution order: explicit `explorer_api_url` > `Network::default_explorer_base_url`, which is mainnet-only > `None`). Every read and sync call site threads that `None` through candidly instead of hitting the wrong chain: `read_name_info` returns an actionable "No explorer is available for this network" error, `read_balance` falls back to the cached balance, `discover_owned_names` / `read_name_bids` return an empty result, and the background sync / repair steps record an "explorer unavailable — configure `explorer_api_url` or wait for the local node to sync" note rather than seeding the cache from a foreign index.

### Changed
- **The wallet now says when an action is sent but not yet mined.** Every status it showed was derived from the chain's view of a name, and between broadcasting a transaction and the next block that view has not moved — so a name you had just bid on still read "Ready to Bid", and one you had just opened still offered "Open". On a chain that only produces a block when asked (regtest), that gap lasted indefinitely. A transaction of yours still in the mempool now takes precedence: the name modal says "<Action> is broadcast and waiting to be mined", the auctions list shows "Bid · waiting for a block" instead of a phase label that has not caught up, and Activity's status badge explains what each state means and what it is waiting on.
- **Hover hints are now the app's own tooltip rather than the browser's.** All 46 native `title` attributes became the `Tooltip` component: styled, positioned to stay inside scrolling containers, keyboard- and screen-reader accessible, and — the reason for the change — visible on disabled controls, where a `title` never was. Hints wait 300ms before appearing so sweeping the pointer across a toolbar or a table row no longer flashes one per element, and close immediately.

### CI / tooling
- `npm run lint:native-title` fails the build on a native `title` attribute, so hover hints cannot drift back off the `Tooltip` component.

## [0.5.0] - 2026-08-20

> **Highlights:** Hardware wallet support (Ledger Nano S/S Plus/X), keyboard-first navigation with command palette, receive-address list, fee-rate control, and batch-bid operations.

### Added
- **Hardware wallet support — Ledger Nano S / S Plus / X** — connect a Ledger device and let it sign every transaction. Private keys never leave the device; you confirm each send, bid, or covenant action on-screen. Requires the official [`ledger-app-hns`](https://github.com/handshake-org/ledger-app-hns) firmware. Backend: new `providers/ledger/` module with APDU protocol, HID transport, multi-step signing, and device identity verification. Frontend: hardware wallet profile type in AddWalletForm, secure-window confirmation flow, and error mapping for disconnect/timeout scenarios. Database migration `026_ledger_hardware_profiles.sql`. See [docs/LEDGER.md](docs/LEDGER.md) for the full spec.
- **Fee-rate control** — a global default fee rate in Settings > Advanced (`fee_rate_doos_per_kvb` setting, in doos per 1000 vbytes) plus a per-transaction override widget (`FeeRateOverride`) shown in every transaction flow: Send, Batch Renew/Reveal/Redeem/Finalize, single Bid (Name Actions modal), and Batch Bid. The override is a collapsible "Advanced" disclosure with validation, min-value clamping (1000 doos/kvB = 1 sat/byte), and inline help. The Rust backend's `resolve_fee_rate` now reads the setting before falling through to `estimatesmartfee` or the relay-floor default. New shared library: `src/lib/feeRate.ts` (parseDoosPerKvb, doosPerKvbToSatsPerByte, parseFeeRateArg).
- **Batch bid operations** — select multiple names on the Auctions page and open a batch-bid modal showing the count and estimated total fee with a collapsible name list before signing + broadcasting. Reduces friction for bidding on many names at once.
- **Receive address list.** A "View all addresses" disclosure in the Receive card expands a scrollable list of every derived receive-branch address. Each row shows the derivation index, truncated address (with full-address tooltip), a used/fresh badge, the first-seen date, a copy-to-clipboard button, a QR-code toggle, and a mainnet explorer link. A "Generate new address" button at the bottom allocates the next unused receive index. Backend: new `list_receive_addresses` + `reveal_next_receive_address` Tauri commands. The "used" predicate (UTXO OR bid_commitment) is defined once (`derivation::ADDRESS_USED_PREDICATE`) and shared between address allocation and the list query.
- **Keyboard-first navigation.** A cheatsheet overlay (Shift+?) documents every binding, and every panel now has route-scoped shortcuts so common flows never need the mouse:
  - **Command palette** (⌘K / Ctrl+K) — fuzzy-searchable list of the navigation targets and view actions available on the current page. Type to filter, ↑/↓ to move, Enter to run, Esc to close. Write-only actions (Send, Batch Bid) are hidden on read-only wallets so you can't open a dead-end flow.
  - **Wallet (`/`)** — `s` open Send, `r` refresh (Sync), `u` toggle lock/unlock, `q` toggle the receive QR, `/` focus the name filter, and `j`/`k` + `Enter` to walk the Owned Names list and open the selected row's Name Actions modal without touching the mouse.
  - **Auctions (`/auctions`)** — `/` focus the lookup input, `b` open the Batch Bid modal.
  - **Activity (`/activity`)** — `/` focus the search input.
  - **Watchlist (`/watchlist`)** — `a` focus the add-name input, `e` export CSV.
  All action keys are suppressed while an input is focused or a modal is open (the palette itself layers above dialogs), so shortcuts never hijack typing. The cheatsheet groups bindings by category and filters action/list keys down to the ones that actually work on the current page, keeping the reference honest.
- **DEV: Simulate update flow** — a dev-only "Simulate update available" panel in Settings (gated behind `import.meta.env.DEV && isTauri()`) seeds the shared `useAppUpdate` store from the latest GitHub release (or a synthetic bumped version when offline) so the banner + Settings card show the "available" notice without auto-installing. Clicking "Install now" then runs a fake download loop (10 ticks × 120 ms → installed) via a `simulated` flag on the store, so the full update UX can be exercised without a real signed release. New Rust command: `fetch_latest_release_meta` (debug-gated).

### Fixed
- **Sync no longer wipes balances when the node is unreachable.** If every per-address coin query errors (node missing `--index-address`, or simply offline mid-sync), `sync_node_step` now bails early instead of marking all tracked UTXOs as spent — preventing a false "everything spent" balance wipe and cached-name loss on transient node failures.
- **"Launch at login" no longer starts the wrong build.** If you'd ever enabled launch-at-login from a development build, macOS would keep starting that stale dev binary at login instead of the installed Namehold app — showing a blank window and the wrong Dock icon. Autostart is now registered only in release builds, and the "Launch at login" toggle is disabled in dev builds, so a development session can no longer hijack your login item. (If you hit this, delete `~/Library/LaunchAgents/Namehold.plist`, then re-toggle "Launch at login" from the installed app to re-register it correctly.)
- **Activity page degrades gracefully without an address index.** `read_action_history` now swallows per-address history failures and shows an empty "No activity yet" state instead of surfacing an error toast when the local hsd node lacks `--index-tx` / `--index-address`.
- **Inline draft actions in Activity table.** Draft rows (unsigned, failed, signed-but-unbroadcast) now show contextual "Sign & broadcast" / "Broadcast" / "Retry" and "Discard" buttons directly in the row, so you can act on pending drafts without navigating away. The Txid cell no longer renders a disabled button when there's no txid — it shows a plain dash instead.
- **`S` (and other write-gated) keyboard shortcuts now explain themselves.** Pressing `S` on the Wallet page to open Send used to do nothing at all when the wallet couldn't send yet (read-only wallet, locked signer, or coins not synced) — no modal, no message. It now surfaces the same guidance the disabled Send button shows (e.g. "Unlock your wallet to sign transactions.") as a toast, so the key never silently no-ops.
- **Actions column stays single-line.** The new Actions cell uses `whitespace-nowrap` so buttons never wrap to a second line.

### CI / tooling
- **Fix flaky apt-get update timeout in CI.** The `Refresh apt package index` step was hard-failing when `apt-get update` timed out on a throttled Ubuntu mirror (observed on run 32273872139: rust-lint hit the 90s cap on all 3 retries while rust-test fetched the same 11 MB index in 54s on a healthier mirror). Made the step best-effort (no `exit 1`) so a slow mirror can't flake the job on the common cache-hit path. Per-attempt timeout raised 90s → 120s. Applied to both CI and release workflows.
- **Add libudev-dev to CI for hidapi crate.** The `hidapi` crate (required for Ledger HID transport) needs `libudev` system headers to compile on Linux. Added `libudev-dev` to the apt install step in both rust-lint and rust-test jobs, with cache version bump to invalidate stale cached packages.
- **Add PR auto-labeler workflow** (actions/labeler v7). New `.github/workflows/labeler.yml` + `.github/labeler.yml` ruleset automatically labels PRs by path patterns (rust, frontend, ci, docs, tests, etc.) so reviewers can filter and triage at a glance. Uses actions/labeler v7 (Node.js 24 native, no deprecation warnings on GitHub-hosted runners).
- **Ledger device simulator for testing.** A `mock-ledger` Cargo feature plus the `NAMEHOLD_LEDGER_SIM` env var swap in an in-process simulated HID transport (`src-tauri/src/providers/ledger/simulated_hid.rs`) that scripts reject, timeout, wrong-app, and disconnect scenarios — so Ledger error paths can be exercised in CI and locally without physical hardware.
- **Comprehensive test coverage improvements (PR #50).** Improved test coverage for v0.5.0 across both stacks: (1) **RPC trait injection** — introduced an `async_trait` `NodeRpc` trait as an injection seam, letting us test soft-degrade paths, error branches, and phase-dependent decisions with plain in-memory mocks instead of requiring a live hsd node or HTTP mock server. (2) **Draft-command extraction** — extracted all 9 `build_*_draft` Tauri commands into pure sync `_inner` functions testable without a Tauri runtime, each taking `&Connection` + pre-fetched inputs. This makes the core transaction-building logic directly testable with an in-memory SQLite DB — sub-millisecond per test. (3) **Test suite expansion** — 54 direct unit tests for the 9 draft commands, 78 additional pure-logic and mock-based tests for name-action capabilities, fee-rate, and context helpers, plus 5 new frontend test suites (Watchlist, WalletView action-bus, Settings fee-rate, AuctionsView batch-bid, error_tests wire-up). Total: **1405 Rust tests** (up from 1288) + **745 frontend tests** across 73 files, with **81.28% line coverage** on the Rust backend.

## [0.4.1] - 2026-08-11

A polish release focused on the tray, the auto-updater, and getting
launch-at-login working on macOS.

### Added
- **Cleaner menu-bar mode on macOS.** Close Namehold to the tray and the
  Dock icon disappears — you get a proper menu-bar-only experience while
  the app keeps running in the background. Bring the window back and the
  Dock icon returns. Clicking the Dock icon (or picking Namehold from the
  app switcher) now reliably reopens the window, and Cmd+Tab still works
  the whole time.
- **First-time "still running" hint.** The first time you close Namehold
  to the tray, a native notification lets you know the app is still alive
  in the menu bar and how to get it back. Shown once, then never again.

### Fixed
- **Tray now shows the right node status.** If hsd was already running
  when Namehold launched — say, a previous background-sync session left
  it up, or you started it yourself — the tray used to be stuck on
  "Node: Stopped / Start Node" even though the node was clearly working.
  Clicking "Start Node" would then try to launch a duplicate. The tray
  now watches the node's actual health and stays in sync with reality,
  even when the main window is closed.
- **"Launch at login" actually works now.** Toggling it in Settings used
  to fail with a permissions error. Fixed — the app will now correctly
  start with your Mac / Windows / Linux session when you enable it.
- **Nicer update flow.** The "What's new?" link in Settings now opens the
  same release-notes modal as the top banner (instead of dumping the
  notes inline). Links inside release notes — including relative ones
  like `docs/RECOVER_LOST_BIDS.md` — open in your browser correctly.
  Every button, link, and close-icon in the update flow shows a pointer
  cursor. And once an update is installed, "Restart now" can no longer
  be dismissed away by accident — restart is the only thing left to do.

### Under the hood
- Faster release builds in CI, and fixes to the macOS universal build
  and Windows installer so every release actually makes it out the door.
- Windows build fix for a type-comparison error that was breaking CI.

## [0.4.0] - 2026-08-07

### Added
- **System tray / menu-bar presence** — Namehold now lives in the system tray
  so it keeps running (local hsd node + background sync daemon alive) when the
  main window is closed. The tray menu offers **Open Namehold**, a live node
  status label with a **Start/Stop node** toggle, a **Sync in background**
  checkbox, and **Quit**. A new **System Tray** section in Settings adds two
  toggles: **Close to tray** (default ON — closing the window hides it instead
  of quitting; click the tray icon or use Open to restore) and **Launch at
  login** (registers the app to auto-start, via `tauri-plugin-autostart`:
  LaunchAgent on macOS, Run key on Windows, `.desktop` on Linux). The tray
  icon reflects node state with three variants (normal / syncing / stopped),
  rendered as a macOS template image so it adapts to light/dark menu bars. A
  3-second reconciliation ticker keeps the tray in sync with changes made
  outside it (frontend actions, hsd autostart, sync transitions). New Tauri
  commands: `is_close_to_tray_enabled`, `set_close_to_tray_enabled`. New
  settings keys: `close_to_tray`, `launch_at_login`.

- **Recover lost bids from any hsd wallet** — if you reinstall, seed-restore,
  or import a bid from another hsd-compatible wallet, Namehold can recover the
  bid value without you remembering the exact amount. The Name Actions modal
  shows a **Recover bid** panel during the REVEAL phase with two options:
  enter the amount if you remember it, or click **Auto-recover (brute-force)**
  to sweep candidate values. Recovery uses only the account xpub (public) —
  never needs your passphrase — and works because the nonce derivation is the
  hsd standard, not Namehold-specific. Typical bids under 100 HNS recover in
  seconds. See [docs/RECOVER_LOST_BIDS.md](docs/RECOVER_LOST_BIDS.md) for the
  full guide.

- **Background HSD Sync Daemon** — a separate Rust binary (`namehold-syncd`) that
  syncs wallet profiles (UTXOs, name states, transactions) from the local hsd node
  every 60 seconds, even when the app is closed. Controlled by a Settings checkbox
  **"Sync in background"** (default ON). When enabled, hsd stays running after app
  exit; the next launch adopts it. A cross-process DB lock table (`sync_locks`)
  coordinates the app's manual Sync and the daemon via heartbeats (10s) and
  stale-lock takeover (30s) to prevent concurrent writes. Crash recovery: the app
  respawns the daemon on startup if the toggle is ON and the daemon is dead.
  Bundled as a Tauri sidecar (externalBin).
- **New DB migration `021_sync_locks.sql`** — creates the `sync_locks` table for
  cross-process sync coordination.
- **SPV (Simplified Payment Verification) mode** — an opt-in lightweight alternative
  to the full node mode. SPV downloads only block headers (~几十MB vs ~15GB),
  enabling fast first launch and minimal disk usage. Balance and name data come from
  the explorer; sending is blocked in SPV mode (read-only). Controlled via a
  **"Node mode"** dropdown in Settings → Connections (default: Full node).
- **Explorer failover** — all explorer HTTP requests now support automatic failover
  to a configurable fallback URL. Configured via "Explorer fallback URL" in Settings.
- **SPV UI indicators** — StatusStrip shows "Explorer (SPV)" when in SPV mode to
  explain why reads come from the explorer.
- **SPV-aware write capability** — SPV mode shows a clear "SPV mode cannot send
  transactions" message instead of the generic "node not address-indexed" error.
- **TLD Management P1 — Batch operations** — renew, reveal, redeem, or finalize
  multiple names in one transaction. Multi-select checkboxes on the Owned Names
  table with a batch action bar ("Renew Selected" / "Reveal Selected" /
  "Redeem Selected" / "Finalize Selected"). Each action opens a
  `BatchConfirmModal` showing the count and estimated fee with a collapsible
  name list before signing + broadcasting.
- **TLD Management P2 — Watchlist** — track names you don't own for monitoring.
  New "Watchlist" page in the sidebar with add/remove, tags (comma-separated),
  bulk state fetch via `get_watchlist_status`, CSV import/export (`name,tags,
  notes,added_at,state,expiry`), and an "Add to Watchlist" toggle in both
  `NameActionsModal` and `NameInfoModal`. Database migrations
  `022_watchlist.sql` (base table) and `024_watchlist_tags.sql` (adds `tags`
  column).
- **TLD Management P3 — Atomic paid name swaps** — atomic finalize-with-payment
  covenant: the buyer finalizes a TRANSFER and pays the seller in the same
  transaction, so no party can renege after the lockup expires. Buyer side:
  "Buy with payment" button on names in TRANSFER state. Seller side: "Sell
  with payment" flow with saved offer tracking (`023_paid_swap_offers.sql`)
  and a verify-only `claim_paid_transfer` command that inspects the
  broadcast tx before marking an offer paid.

### Changed
- **Removed the Portfolio page and the "Show Portfolio in the sidebar"
  setting.** The hidden `advanced_mode`-gated Portfolio workspace (Inventory,
  Batches, Renewals, DNS tabs) and its `/portfolio` route have been removed
  entirely, along with the `advanced_mode` setting key. The sidebar no longer
  has any advanced-mode gating.

- **MIT LICENSE** — repo now carries an explicit MIT license file.

- **`get_resource` Tauri command** — returns combined name info + DNS resource
  records from the local hsd node with graceful degradation to the explorer for
  name state. Currently backend-only (no frontend consumer after the DNS page
  removal in a later commit).

- **Watchlist v2 — richer columns + background alerts.** The Watchlist table
  now shows three additional columns pulled from the live name info: a
  **Countdown** to the next phase transition (e.g. "Bidding closes in 42
  blocks (~7h)"), the **Highest bid** so far, and **Expires** (days-until-
  expire, colour-graded like the Renewals view). Names owned by the active
  wallet profile get an inline **Owned** badge next to the name. Combined
  with a new opt-in **Watchlist notifications** section in Settings, the
  background sync daemon (`namehold-syncd`) polls each watched name every
  60s, diffs against the last-seen snapshot in a new `watched_name_states`
  cache table, and fires OS notifications on: entry into **BIDDING**, a
  previously CLOSED name becoming **available again**, **bidding-soon** lead
  time (default 144 blocks / ~1 day), and a configurable **global
  highest-bid threshold** crossing (in HNS). Adaptive polling skips names
  whose next transition is > 300 blocks out when the state was refreshed
  within the last 5 minutes. The daemon is the sole notifier — the in-app
  scanner still owns reveal/renewal deadlines for names you've bid on or own,
  so there's no double-fire. Alerts fire even when the Namehold app is closed
  (as long as background sync is enabled). New Tauri command:
  `get_watched_states` (read-only, hydrates the columns without RPC on first
  page open — currently unwired on the frontend; columns hydrate via
  `read_name_info`). New settings keys: `watchlist_notify_enabled`,
  `watchlist_notify_bidding_soon_lead_blocks`,
  `watchlist_notify_highest_bid_threshold_hns`. New DB migration:
  `025_watched_name_states.sql`. New crate: `notify-rust` (cross-platform
  OS notifications, no Tauri AppHandle required).

- **Unified input/button sizing.** `Input` and `Select` now accept an
  `inputSize` prop (`"sm" | "md"`, default `"md"`) that shares the exact
  padding + text-size tokens with `Button`'s `size` prop, so a control and the
  button beside it render at identical heights. All `Button` variants now carry
  a (transparent) 1px border so bordered inputs and borderless buttons line up
  to the pixel. Replaced ad-hoc raw `<input>`/`<select>` elements across the
  Auctions, Activity, Namebase dashboard/import, wallet name filter, DNS
  records editor, and the DataTable search box with the shared `Input`/
  `Select` components. Fixes the visible ~10px height gap between the
  add-name/look-up inputs and their adjacent buttons.

- **Inline node lifecycle actions.** The StatusStrip node pill now opens a
  popover menu with **Start node** / **Stop node** / **Re-sync chain** actions
  (plus an "Open Settings" escape hatch), replacing the old Settings link. The
  WalletView "needs node sync" callout has an inline **Start node** button
  with an "Open Settings" fallback on failure. The update-installed banner
  shows a **Relaunch now** button instead of directing users to Settings.
  New reusable `Popover` component (`src/components/ui/Popover.tsx`).

### Fixed
- **macOS notification sender identity** — OS notifications from the
  background sync daemon (`namehold-syncd`) now attribute to **Namehold**
  instead of "Terminal" / Finder / a generic sender. Because the daemon runs
  unbundled (no Tauri `AppHandle`), it now claims the Namehold bundle ID via a
  `Once`-guarded `ensure_notify_identity()` before emitting; the Tauri app
  pre-empts the same identity before the notification plugin initializes. A
  new **Debug Notifications** panel in Settings (debug builds only) fires each
  notification path on demand for verification, backed by the
  `#[cfg(debug_assertions)]` `simulate_notification` Tauri command.
- **TldInventory bulk Transfer/Finalize** — bulk actions on N selected names
  now actually operate on all N, not just the first. Transfer loops N single
  transactions (per-name recipient safety); Finalize uses the batch draft
  command for a single atomic transaction. (Historical: the Portfolio/
  TldInventory workspace this fixed is removed in this same release.)
- **Layout badge color inversion** — the wallet capability badge now renders
  CAN SEND in green (safe) and READ-ONLY in neutral gray, matching intuitive
  traffic-light semantics (previously CAN SEND was red).
- **Sync UI view jumping** — automatic sync (every 60s) no longer expands the
  full sync status panel. Auto-sync shows "Syncing…" on the button with a
  spinner; the full progress panel only appears for manual Sync.
- **Stale-data banner suppressed when node is live** — the "Couldn't verify
  urgent auction tasks — data may be stale." banner no longer appears during
  transient query hiccups while the node is synced; it only shows when the
  node is actually offline.
- **Unicode/IDN name lookup** — the "Get a TLD" input now accepts Unicode
  characters (e.g. `сбер`, `münchen`) and encodes them to ACE (Punycode) at
  lookup time, instead of silently stripping non-ASCII input. Added `tr46`
  UTS-46 processing library and `src/lib/idnEncode.ts` module.

### Removed
- **Dead migration UI** — `SyncVerification` and `MigrationAssistant`
  components (plus their test) and the `compare_inventory_with_provider`
  backend command have been removed. These were scaffolding for a one-time
  Namebase migration flow that is no longer needed.

### CI / tooling
- **sccache + mold linker + debuginfo thinning** — CI cold-compile speedup
  via distributed compilation caching (`sccache`), the `mold` linker for
  faster linking, and stripped debuginfo in CI builds.
- **Parallel rust-lint + rust-test jobs, nextest adoption** — lint and test
  run concurrently; `cargo-nextest` replaces `cargo test` for per-test
  parallelism and structured output.
- **Fast/full lane split** — PR pushes run a fast lane (subset of tests);
  pushes to `main` run the full suite. Configured via `nextest.toml`.
- **sccache resilience** — graceful degradation when the GHA cache backend
  is unreachable; CI continues uncached rather than failing.
- **Argon2id test-mode cost reduction** — `cfg(test)` drops KDF from 256 MiB
  to 8 MiB, cutting vault-related tests from ~50s to <0.25s each.
- **Per-process temp DB paths** — fixes nextest parallelism flakes where
  concurrent test processes collided on the same SQLite file.

## [0.3.0] - 2026-07-29

### Changed
- **Unified table design** — all tables (Owned Names, Activity, Auctions, Renewals, Batches, DNS records, TLD inventory, Namebase dashboard) now share compact rows, consistent typography, and monospace values. Activity gained a dedicated Block column.
- **Shakeshift explorer** — all explorer links (names, txids, addresses, block heights) now open on Shakeshift (https://shakeshift.com) instead of HNSFans.
- **Release notes from CHANGELOG** — in-app update banner and GitHub releases now show the CHANGELOG entry instead of auto-generated PR titles.

### Added
- **About page** — reachable from the ℹ️ icon next to the version number. Shows logo, version, description, and a link to report issues or request features on GitHub.
- **Branded startup spinner** — animated spinner with app name instead of a blank white screen on launch.
- **Inline "Unlock" buttons** — locked-wallet notices (name actions, send, bid, finalize) now have an Unlock button so you can unlock in place without navigating away.
- **"What's new?" release notes** — the update banner shows a "What's new?" button that opens the release notes in a formatted modal.
- **Unicode-aware name search** — search by the Unicode form (`.münchen`) or the punycode form (`xn--…`) and find the same name.
- **Reveal in-flight UI** — after broadcasting a reveal, the modal shows a pending-confirmation card with txid and explorer link. Auctions rows update automatically without manual refresh.
- **External link opener** — "View on explorer" buttons open the system browser directly.

## [0.2.1] - 2026-07-25

### Changed
- **Always prefer freshest node data** — a background auto-sync now refreshes
  cached data every 60s while the local node is live and synced (kicked on the
  explorer→local edge and on mount if already live), reusing the idempotent
  `start_full_sync` and skipping while a run is in flight. Balance queries are
  no longer sticky: dropped `staleTime: Infinity`/`gcTime: Infinity`/
  `refetchOnMount: false` in favour of a 15s `staleTime` plus a node-gated 20s
  `refetchInterval` (per-profile query keys preserved, no cross-wallet bleed).
  The DNS editor in the Name Actions modal now seeds and enables UPDATE only
  from a guaranteed-fresh on-chain read (`recordsFresh` gate), so a stale base
  can never overwrite the resource.

### Security
- Encrypt the Namebase session cookie at rest under an OS-keyring-held DEK
  (AES-256-GCM). The cookie is stored as a hex-encoded blob in the new
  `namebase_cookie_v1` setting; the plaintext `namebase_cookie` setting is
  blanked on migration and on disconnect (defense in depth). Existing users'
  plaintext cookies are migrated transparently on first read.
- Add `SECURITY.md` at the repo root documenting the full threat model, per
  attack-surface mitigations, residual risks (honest disclosure), the
  lower-risk manual-transfer alternative, and a reference table mapping each
  concern to the enforcing code + tests.
- Redact sensitive settings (`namebase_cookie`, `node_rpc_api_key`,
  `hsd_api_key`) from `get_settings`; the renderer now sees only
  `__has_<key>` presence markers, never the raw value.
- Enforce a host allowlist on the Namebase API base URL, require HTTPS for
  the real Namebase host (no cleartext), and treat the `namebase_base_url`
  setting as a debug-only test seam — it is ignored in release builds.
- Deny renderer writes to security-critical settings (`namebase_base_url`,
  `namebase_cookie`) via `update_setting`.
- Require explicit user confirmation in the Rust-owned secure window before
  signing any draft (`sign_tx_draft`). The confirmation window shows the
  action, recipient, amount, fee, txid, and any warnings so a compromised
  main webview cannot swap details silently.
- Redact sensitive values in `audit_log`; `get_audit_log` also re-redacts
  legacy plaintext rows on read (defense in depth).
- Refuse to send an RPC api-key over plaintext HTTP to a non-loopback host;
  `NodeRpcClient::new` blanks the key defensively when misused.
- Ship a restrictive Content Security Policy for the Tauri webviews.

### CI / tooling
- Add a CI dependency-audit gate that runs `cargo audit --deny warnings` and
  `pnpm audit --audit-level moderate --prod` on every PR, surfacing
  newly-disclosed advisories in the dependency graph.
- Suppress reviewed, not-applicable/unavoidable advisories with justification:
  frontend via `auditConfig.ignoreGhsas` in `pnpm-workspace.yaml`, backend via
  `[advisories] ignore` in `src-tauri/.cargo/audit.toml`. Each entry is
  documented in `SECURITY.md`; a new advisory not on the list still fails CI.
- Bump `anyhow` to `1.0.104` to clear RUSTSEC-2026-0190 (unsound
  `Error::downcast_mut`) rather than suppress it.
- Pin pnpm to `11.17.0` across CI (`pnpm/action-setup`) and add a
  `packageManager` field so local (Corepack) and CI use the same version.
  Audit config moved out of the (now-ignored) `package.json` `pnpm` field into
  `pnpm-workspace.yaml`, its home under pnpm 11.
- Add a `lint:secure-imports` check ensuring nothing under `src/secure/**`
  imports from the rest of `src/`, keeping the secure-window bundle isolated.
- Harden `.gitignore` with explicit secret patterns (`*.pem`, `*.key`,
  `id_rsa`, `secrets.*`, `credentials.*`).

### Fixed
- README and USER_MANUAL privacy claim: the app is local-first, not
  local-only. The docs now spell out that the HNSFans explorer sees wallet
  addresses and tracked names by default and how to run a local hsd node
  for fully local lookups.

## [0.2.0]

### Added
- **Node-only reads + chain scanner** — owned names, balances, and per-name bid
  history read directly from the local hsd node when synced, eliminating explorer
  dependency for synced wallets. A background chain scanner indexes BID/REVEAL
  outpoints for honest bid display.
- **DNS record prefill** — the Manage DNS editor in the Name Actions modal now
  prefills existing on-chain records (`getnameresource`) so you can edit rather
  than re-enter from scratch.
- **Autostart hsd** — the app starts hsd on launch by default (toggleable in
  Settings → Connections). If hsd is already running, it adopts the existing node.
- **Message signing** — sign an arbitrary message with the wallet key that owns a
  name (proves name ownership off-chain).
- **Richer DNS editor** — real hsd record types (DS, GLUE4/6, SYNTH4/6) with
  multi-field editing; raw-JSON advanced toggle.
- **Owned-names filter** — substring search for the Owned Names list.
- **Per-bid detail in auctions** — `NameBidsPanel` shows individual bids with
  lockup/revealed values, marks your own bids, and computes an honest "highest"
  (only revealed values count).
- **Active Auctions view** — names with an open auction position merged into a
  live-phase list; pending-OPEN surfacing; double-open guard.
- **Auto-update** — the app checks GitHub Releases ~30s after launch (and on
  demand from Settings → Updates), then downloads and installs signed updates
  in place. Update bundles are Ed25519-signed at release time and verified
  against the embedded public key before install. See `docs/RELEASING.md`.

### Changed
- **WalletView density polish** — CopyField, Disclosure, and truncateMiddle
  primitives; xpub collapsed by default; balance cards consolidated.

### Fixed
- Recent transactions Amount now shows net cost (bid value), not the name's
  total locked value.
- Bid commitments now persist `bid_txid`/`reveal_txid` at build time (+ backfill
  for pre-fix commitments) so own-bid marking is reliable.

## [0.1.0] - 2026-06-30

### Added
- Non-custodial wallet: HD key derivation (BIP39/BIP32), transaction building, signing, address generation
- hsd chain backend: direct RPC to hsd node for balance, names, transactions, and mempool
- Name operations: register, transfer, renew, update, redeem, finalize
- Auction flow: bidding, revealing, and domain lifecycle tracking
- Namebase integration: import domains from Namebase, bulk transfers, renewal calendar
- hsd node control: start/stop/restart, one-click re-sync, index-setup adaptation
- Multi-provider read architecture with advanced and onboarding flows
- App shell with navigation, settings, and wallet lifecycle management
- Transaction confirmation tracking and send-max support
- Domain expiry monitoring and renewal reminders
- QR code display for receiving addresses
- CI and release workflows for automated cross-platform builds
