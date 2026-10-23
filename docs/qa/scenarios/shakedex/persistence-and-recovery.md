---
area: Shakedex across restarts, node outages, unsent drafts and background jobs
priority: P1–P3
blocks: ~15
requires:
  - environment.md
  - procedures/lock-and-list.md
  - procedures/cli-fill.md
  - procedures/mine-and-sync.md
last_verified: "bc8cd4d, 2026-10-18, report kept outside the repo"
---

# Persistence and recovery

What survives a quit, what an unsent transaction looks like on its row, what happens when the node is down, and what the background jobs do without a Sync. Quit the app with its menu's Quit or `osascript -e 'tell application "Namehold" to quit'` ([ui](../../ui.md#known-quirks-and-their-workarounds)).

## X1 — A restart mid-flow; an unsent lock left on its row

- **Priority:** P1
- **Was:** full QA X1; QA_WORKFLOWS §J (Send and Discard on a row)
- **Covers:** R19, R23
- **Preconditions:** an ordinary owned name (SA before S2, or SE before R1).

Steps:

1. (a) Lock the name, `mine 1`: `Locking · 9 blocks`; quit and relaunch. (b) Likewise once a row is `Listed for sale` and once it is `Cancelling` (fold into S2).
2. (c) `Sell…` → `Lock name` (the draft is built) → quit instead of `Sign and send the lock` → relaunch, unlock → My listings, Owned Names, Activity.
3. `Discard` on the row.
4. Repeat step 2, then press the row's `Sign and send` → secure window → confirm → `mine 1`, Sync.

Expected:

- (a), (b): the state, the block count and the `My listings` button survive the restart.
- (c): the row reads `Locking` (no block count: nothing is mined), mode `Buy now`, with `Sign and send` and `Discard` beside a disabled `Finalize & sign`; Owned Names shows the listing badge; Activity keeps the `Lock for sale` draft.
- Step 3: toast "Lock discarded. Nothing was sent: the name stays yours."; the row goes; the name is ordinary; the balance unchanged.
- Step 4: toast "Sent", then `Locking · 9 blocks`.
- Passes if the unsent lock survives a restart and can be both discarded and sent. If the draft did not survive the quit, record it with what you saw.

- **Blocks mined:** 2
- **Evidence:** screenshots `X1-c-unsent-row`, `X1-c-discarded`, `X1-c-sent`.
- **Cleanup / end state:** the name `Locking` (continue with its case) or ordinary.

## X2 — The R19 abort: a Cancel transfer of a Locking listing, unsent and sent

- **Priority:** P1
- **Was:** full QA X2; T7 S6; T7 recheck F5; QA_WORKFLOWS §J (the R19 abort)
- **Covers:** R19
- **Regression of:** F5 (Finalize & sign gave the lockup reason while a Cancel transfer of the listing was pending)
- **Preconditions:** a `Locking · N blocks` listing (after the lock's `mine 1`).

Steps:

1. `Manage` → `Cancel transfer` → the secure window opens → quit the app while it is open → relaunch, unlock; read My listings and Activity.
2. Optionally `mine 9`, Sync, and read again (the lockup over).
3. In Activity, `Discard` the Cancel transfer draft; read My listings.
4. Either finish the listing (Finalize & sign, the case it belongs to), or: `Cancel transfer` → sign and send → **do not mine** → Sync → My listings; then `mine 1`, Sync.

Expected:

- Step 1: if the built draft survived, `Finalize & sign` is disabled with "a Cancel transfer of this listing is pending and spends the same coin: Finalize & sign waits until it is dropped, failed or deleted" (not the lockup sentence); Activity shows the draft with `Sign & broadcast` and `Discard`. If it did not survive, record what you saw.
- Step 2: the same reason after the lockup.
- Step 3: (a confirm may appear first) the draft goes; `Finalize & sign` is back, disabled only by the lockup reason, or enabled once Ready.
- Step 4, sent and unmined: the same pending reason while the abort sits in the mempool. After the block: `Lock aborted`, no buttons; Owned Names shows the name as an ordinary owned name with no listing badge; `info` shows `transfer 0`.
- Passes if Finalize & sign is never offered while the abort is pending and the abort ends the listing.

- **Blocks mined:** 0–10
- **Evidence:** screenshots `X2-0-unsigned-secure`, `X2-1-pending-abort-reason`, `X2-3-discarded`, `X2-4-lock-aborted`.
- **Cleanup / end state:** the name Listed (finished) or ordinary (aborted).

## X3 — Node down at broadcast; send again

- **Priority:** P2
- **Was:** full QA X3; QA_WORKFLOWS §J (a transaction the node may hold offers Send again and no Discard)
- **Covers:** R28
- **Preconditions:** a `Listed for sale` row.

Steps:

1. `Cancel listing` → `Sign and send the cancel` → when `Confirm cancel` is open, `scripts/regtest.sh stop` → confirm.
2. Read the dialog and the row.
3. `scripts/regtest.sh --with-wallet` (a restart, never a reset), Sync, press the row's `Send`.
4. Optionally the same once for Finalize & sign (stop the node while its secure window is open).

Expected:

- Step 2: a broadcast error followed by "The signed transaction stays in My listings, where it can be sent again." (or "…where it can be sent or discarded."; quote which); the row reads `Cancelling` with `Send` (disabled while the node is offline) and no `Discard` when the node may hold it.
- Step 3: `Send` enabled after the Sync; it needs no new signature; toast "Sent"; the transaction reaches the mempool and the row carries on (`Cancelled · finalize to bring it home` after a block).
- Passes if the signed transaction is kept and sent once the node is back.

- **Blocks mined:** 0 (plus the cancel cycle's own)
- **Evidence:** screenshot `X3-1-send-again`.
- **Cleanup / end state:** the node running; the listing in its cancel cycle.

## X5 — Node down for reads and sync

- **Priority:** P2
- **Was:** full QA X5
- **Covers:** R6, R7, R13, R19
- **Preconditions:** rows in `Listed for sale`, `Locked — listing details unknown` and a purchase `Ready to finalize` (or awaiting); a listing file at hand.

Steps:

1. `scripts/regtest.sh stop`.
2. Sync; open My listings; Browse → `Import listing…` → import a file; select a row imported earlier and read its panel; `Sell…` → `Lock name` on an ordinary name; `Restore` a name.
3. `scripts/regtest.sh --with-wallet`, Sync; read every row again.

Expected:

- Sync reports an error and stops (for example "No explorer is available for this wallet's network, and the local node is not synced. Set 'explorer_api_url' in Settings, or wait for the node to finish syncing.", shown briefly) and the Sync button comes back.
- Listed rows read `Current price could not be checked: your node did not answer. Is it running? Check Settings → Connections.` (or the timeout sentence "your node took too long to answer. Try again in a moment."); no row changes state, nothing is aborted, expired or lost.
- The import dialog says "Couldn't check it: your node did not answer. Is it running? Check Settings → Connections." and adds no row; the earlier row's panel cannot price the purchase: the line above its disabled Buy gives a node reason; `Lock name` and `Restore` fail with a node reason; name actions may show "Name actions unavailable." with the node's reason and `Sell…` disabled. Nothing is written: no new draft in Activity.
- After the restart and a Sync every row is as before.
- Passes if an outage changes no state and writes nothing.

- **Blocks mined:** 0
- **Evidence:** screenshots `X5-1-price-not-checked`, `X5-2-buy-preview-offline`, `X5-3-after-restart`.
- **Cleanup / end state:** the node running, the same chain height.

## J1 — States move without pressing Sync

- **Priority:** P2
- **Was:** full QA J1
- **Covers:** R13, R19
- **Preconditions:** any `mine` that should change a state (a lock's `mine 9`, a purchase's `mine 1`).

Steps:

1. After the `mine`, do not press Sync; watch My listings and Owned Names for up to 3 minutes.

Expected:

- Record whether and when the row changes on its own (the app's periodic sync). Not moving within 3 minutes is ◐ with the time noted, not a bug by itself.
- Passes if the row moves on its own within 3 minutes.

- **Blocks mined:** 0
- **Evidence:** two snapshots with their times.
- **Cleanup / end state:** nothing changed.

## J2 — Reminder notifications

- **Priority:** P3
- **Was:** full QA J2
- **Covers:** R14, R19, R28
- **Preconditions:** Settings › Notifications `Enable deadline notifications` on (don't change OS permissions without need).

Steps:

1. When a listing becomes `Ready to Finalize & sign`, a cancel's lockup ends, and a purchase is `Ready to finalize`, open Notification Center and read only Namehold's group (or ask the maintainer to open it).

Expected:

- Title `Ready to finalize`, with the bodies "<name>: the transfer lockup is over — Finalize & sign to lock it in and set its price", "<name>: the cancel's transfer lockup is over — finalize it to bring the name home" and "<name> is paid for — finalize it to make it yours", one per event (the listing reminder repeats daily while it stays ready).
- Passes if each kind is seen; if Notification Center cannot be read, mark — not run.

- **Blocks mined:** 0
- **Evidence:** the Namehold notification texts only, never the desktop or other apps' notifications.
- **Cleanup / end state:** clear nothing; settings unchanged.

## J3 — The background daemon follows a listing while the app is closed

- **Priority:** P3
- **Was:** full QA J3
- **Covers:** R25 (the jobs run in the daemon), spec §5 "Background jobs need a running process"
- **Preconditions:** Settings → `Sync in background (keep wallet up to date when app is closed)` on (note the original value); a Listed listing exported, of a spare name.

Steps:

1. Check that `namehold-syncd` runs and was started from the installed build (`ps -o lstart= -p $(pgrep -x namehold-syncd)` later than the install); an older process is a leftover, not the build under test.
2. Quit the app; [cli-fill](../../procedures/cli-fill.md) mined at once with the file; wait 2–3 minutes (the daemon syncs about every 60 s).
3. Relaunch, and before any Sync read My listings and Owned Names.

Expected:

- `Sold` already shown at launch, or after the app's first automatic sync (say which; the daemon's effect is ✅ only if it is there before any app sync ran, e.g. with the node stopped before the relaunch).
- The daemon never signs or broadcasts: nothing new in Activity besides what the chain shows.
- Passes if the daemon recorded the sale while the app was closed.

- **Blocks mined:** 1 (+11 to list a spare name)
- **Evidence:** the daemon's start time; screenshot `J3-1-sold-at-launch`.
- **Cleanup / end state:** the background setting at its original value; the node running.

## J4 — The background daemon sends the finalize reminders while the app is closed

- **Priority:** P2
- **Covers:** R14, R19, R28 (the reminders' sender), spec §5 "Background jobs need a running process"
- **Preconditions:** Settings › Notifications `Enable deadline notifications` on and saved; Settings › Connections `Sync in background (keep wallet up to date when app is closed)` on (note both original values); `namehold-syncd` started from the installed build ([J3](#j3--the-background-daemon-follows-a-listing-while-the-app-is-closed) step 1); the maintainer watching the screen, and Namehold allowed to post notifications in macOS System Settings › Notifications (the maintainer's call, never changed by the agent). A name to lock and later cancel: fold the case into [S2](selling.md#s2--buy-now-lock-finalize--sign-listed-cancel-and-bring-home) (part 1 at its step 4's `mine 9`, part 2 at its step 8's `mine 9`), or run it on a spare ordinary name with [lock-and-list](../../procedures/lock-and-list.md) and [cancel-and-bring-home](../../procedures/cancel-and-bring-home.md).

This case exists for one manual check: a macOS notification banner that the daemon, not the app, posts. The banner is outside Namehold's windows, so it is not screenshotted ([rules](../../rules.md#secrets-and-evidence)): the maintainer reads it, or Namehold's group in Notification Center is read, and the report says who saw it. An unsigned build may show the macOS permission prompt for Namehold on its first notification, or post nothing until Namehold is allowed; the daemon's own delivery errors go nowhere visible (its output is discarded).

Steps:

1. Part 1. Lock the name and `mine 1`, Sync: `Locking · 9 blocks`.
2. Quit the app (`osascript -e 'tell application "Namehold" to quit'`). Check `pgrep -x namehold-wallet` prints nothing and `pgrep -x namehold-syncd` still prints a PID.
3. `mine 9`; note the time. Wait up to 3 minutes (the daemon syncs every 60 seconds and sends its reminders right after each sync).
4. Relaunch; before any Sync read My listings. Keep the app open for 15 minutes (the app checks on launch and every ~10 minutes) and watch for another banner about the same listing.
5. Part 2. Settings › Connections: untick `Sync in background (keep wallet up to date when app is closed)` (it applies at once and stops the daemon); `pgrep -x namehold-syncd` prints nothing.
6. Take the listing through Finalize & sign and its `mine 1`, then `Cancel listing` and its `mine 1`, Sync, as S2 does. Then `mine 9`, note the time, do not press Sync; keep the app running (window open or closed to the tray) for up to 15 minutes.
7. Tick `Sync in background (keep wallet up to date when app is closed)` again if it was on; check that `namehold-syncd` runs; bring the name home (S2 step 8, or the procedure).

Expected:

- Step 2: the daemon outlives the app.
- Step 3: within about 3 minutes, with no app process running, one banner from Namehold: title `Ready to finalize`, body "<name>: the transfer lockup is over — Finalize & sign to lock it in and set its price". Only the daemon can have sent it.
- Step 4: My listings already reads `Ready to Finalize & sign` at launch (the daemon's sync wrote it). No second banner for this listing while the app runs: the app leaves the finalize reminders to the daemon while its heartbeat is fresh, and the reminder was claimed once. The listing reminder repeats once per UTC day, so a banner after UTC midnight is expected, not a duplicate.
- Step 6: with the daemon stopped, the app sends instead: within about 15 minutes (its own periodic sync must first count the lockup over; the stopped daemon's heartbeat counts as fresh for 3 minutes; the app checks every ~10 minutes) one banner `Ready to finalize`, body "<name>: the cancel's transfer lockup is over — finalize it to bring the name home"; exactly one. The app's periodic sync is not checked here ([J1](#j1--states-move-without-pressing-sync) is): if no banner arrives within 15 minutes, record ◐ with what was seen (whether My listings moved to `Cancelled · finalize to bring it home` with `Finalize home` enabled on its own, the time and height), then press Sync and wait one more app interval (~10 minutes) before giving up.
- With `Enable deadline notifications` on and macOS refusing them, Settings › Notifications reads "OS notifications are blocked for this app. Enable them in your system notification settings — …": then step 3 and step 6 are BLOCKED ✗, not ❌.
- Passes if the daemon posts the listing reminder with the app quit, the app does not post it a second time, and the app posts the cancel reminder once the daemon is stopped. A banner nobody saw, with Notification Center unreadable, is ◐ with the times noted.

- **Blocks mined:** 0 inside S2 (22 on a spare name)
- **Evidence:** the daemon's start time and PID before and after the quit; who saw each banner, at what time and height; screenshot `J4-4-ready-at-launch`.
- **Cleanup / end state:** both settings at their original values, the daemon running if background sync was on; the name home (or as the host case leaves it).
