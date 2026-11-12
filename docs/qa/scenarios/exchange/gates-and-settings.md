---
area: HNS ⇄ BTC exchange — who may trade (R17), enabling, the relay gate, the Exchange settings and their secure window, the bitcoind login's secrecy
priority: P1–P3
blocks: HNS 0, BTC 0
requires:
  - procedures/exchange-setup.md
  - procedures/switch-profile.md
last_verified: "E4 ◐, E6 ✅ on 8d904e2 (2026-11-09); E5 ✅ on f542de6 (2026-11-10); on 0f6ce83 (2026-11-11): E1 ✅ (no Ledger), E2 ✅, E3 ✅ (no SPV step; the gate refresh ✅ on 3acbf91), E7 ◐ (the inspector step blocked; the refused-login text ✅ on 3acbf91)"
---

# Gates and settings

The Exchange page (sidebar `Exchange`, hotkey `4`, route `/exchange`) asks the backend once whether this wallet may trade (`exchange_overview`) and shows the backend's sentence verbatim: as an amber banner, and as the hover hint of every disabled button (`src/lib/exchangeText.ts`, `GatedButton`). The order of the gates is: the profile and node (R17) → `Enable the exchange first.` → no relay. Hover hints are often not exposed to accessibility ([ui](../../ui.md#driving-the-app)); in mode B the inspector reads them all at once:

```js
// webview inspector (mode B: right-click › Inspect Element), on the Exchange page
[...document.querySelectorAll('[data-refusal]')].map(b => [b.textContent.trim(), b.dataset.refusal])
```

The R17 sentences (`src-tauri/src/commands/exchange.rs::refusal`), checked in this order:

- `NEEDS_RECOVERY_PHRASE`: "The exchange needs a wallet whose recovery phrase is on this computer: a Ledger or watch-only wallet cannot sign a trade."
- `NEEDS_NODE` (the Explorer chain source): "The exchange needs a node: an explorer cannot check a lock."
- `NEEDS_LOCAL_NODE` (any remote node, sending allowed or not): "The exchange needs a node on this computer: a remote node could show a fake lock and learn your secret."
- `NEEDS_SENDING_NODE` (a node that cannot send, i.e. SPV now that a remote node is refused first): "The exchange needs Namehold's local node, or your own hsd on this computer, able to send."

So the exchange profiles connect with Chain source `Local full node (this device runs hsd)` to the regtest node ([exchange-setup](../../procedures/exchange-setup.md) step 6).

## E1 — A watch-only (or Ledger) profile cannot trade

- **Priority:** P1
- **Covers:** R17
- **Regression of:** exchange QA finding 25 (the page read "Couldn't check the exchange", "Invalid input: active profile is watch-only" and its order book never loaded)
- **Preconditions:** [exchange-setup](../../procedures/exchange-setup.md) done (the relay set, so the order book loads); at least one offer on the relay (OS of [E10](offers-and-taking.md#e10--publish-a-sell-offer) or OB of [E11](offers-and-taking.md#e11--publish-a-buy-offer)), else the Take part is — not run. A watch-only regtest profile `QA Exchange Watch` made from the Buyer's account xpub (Wallet › `Receive` › Advanced, then Add wallet → watch-only, [environment](../../environment.md#profiles) step 4); create it once and keep it.

Steps:

1. [switch-profile](../../procedures/switch-profile.md) to `QA Exchange Watch`. Open Exchange.
2. Read the banners, the header buttons (`Refresh`, `Make an offer…`, `Settings`), the `BTC account` card and its `Withdraw…`, the Order book rows' `Take`, the `My offers` and `My trades` tabs. Hover each disabled button (or run the inspector line above).
3. Optional: with a regtest Ledger profile, repeat steps 1–2.

Expected:

- An amber banner: "The exchange needs a wallet whose recovery phrase is on this computer: a Ledger or watch-only wallet cannot sign a trade."
- Under it the blue banner "**The exchange is off for this wallet.** Enabling it stores the public keys of a Bitcoin account made from this wallet's recovery phrase (any BIP 84 wallet restores it from the same phrase) and of the keys your trades use. It needs the wallet unlocked and makes no transaction." with `Enable exchange` disabled.
- `Refresh`, `Make an offer…`, `Settings`, `Enable exchange`, `Withdraw…` and every `Take` are disabled, and every one's hint is that same sentence (never "Enable the exchange first." and never the relay sentence: the R17 gate comes first). The inspector lists only that sentence.
- The BTC account card reads "Enable the exchange to get a Bitcoin account made from this wallet's recovery phrase."; My offers "You have no offers. Make one to let others trade with you."; My trades "No trades yet. Take an offer in the order book, or make your own."
- The order book itself still lists the relay's offers (browsing needs no keys).
- Step 3: the same sentence. Without a device: BLOCKED ✗.
- Passes if nothing on the page can act on the watch-only profile and every refusal is the one sentence.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshot `E1-1-watch-only`; the inspector's list.
- **Cleanup / end state:** switch back to the Buyer.

## E2 — The Explorer chain source refuses the exchange

- **Priority:** P2
- **Covers:** R17
- **Preconditions:** the Buyer, exchange enabled; no trade between its two Commits (a node change mid-trade is not what this case checks). Note the Connections settings before you change them.

Steps:

1. Settings › Connections › `Chain source` → `Read-only (never send)` → `Save settings` → confirm in the secure window.
2. Exchange: read the banner and the buttons (as E1 step 2).
3. Switch to the Seller and look at its Exchange page; switch back. (Whether Connections are per profile is recorded, not judged here.)
4. Settings › Connections › `Chain source` back to `Local full node (this device runs hsd)` with `Node RPC URL (sending)` `http://127.0.0.1:14037` and `Node RPC API key` `test` → `Save settings` → confirm. Exchange again.

Expected:

- Step 2: the amber banner "The exchange needs a node: an explorer cannot check a lock."; `Refresh`, `Make an offer…`, `Settings`, `Withdraw…`, every `Take`, and on My trades every `Confirm and sign` and `Cancel` disabled with it. No Enable banner (the profile is already enabled). The cards of existing trades are still shown.
- Step 3: record whether the Seller is refused too.
- Step 4: no banner; the buttons enabled again.
- Passes if the Explorer source refuses every trading button with that sentence and the refusal lifts with the node back.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshot `E2-1-explorer`.
- **Cleanup / end state:** Connections as noted before step 1.

## E3 — A remote node refuses the exchange, with or without sending

- **Priority:** P1
- **Covers:** R17 (a remote node, review M3: the Buyer reveals on its node's word, so the node must be its own)
- **Regression of:** exchange QA finding 15 (the SPV refusal recommended a remote node, which is refused), exchange QA finding 26 (the page kept the old gate for about half a minute after a Connections change)
- **Preconditions:** the Buyer, exchange enabled, connected as `Local full node (this device runs hsd)`; no trade between its two Commits. Note the Connections settings first.

Steps:

1. Settings › Connections › `Chain source` → `Remote node (point at someone else's hsd)`, `Node RPC URL (sending)` `http://127.0.0.1:14037`, key `test`, `Allow sending via remote node` ticked → `Save settings` → confirm. Exchange: banner and buttons as E1 step 2.
2. Untick `Allow sending via remote node` → `Save settings`. Exchange again.
3. Optional (P3): `Chain source` `SPV — lightweight, read-only (headers only, explorer for data)` (it needs an hsd restart: only if the maintainer agrees), Exchange again.
4. Back to `Local full node (this device runs hsd)` with the URL and key → `Save settings` → confirm. Exchange again.

Expected:

- Read the Exchange page right after each `Save settings`: it must show the new gate at once (the old state lingering for about 30 s is exchange QA finding 26 again). The Connections secure window is `Change connection settings`, its row `Chain source` "<old> → <new>", its button `Save settings`.
- Steps 1 and 2: the amber banner "The exchange needs a node on this computer: a remote node could show a fake lock and learn your secret." (a remote node is refused whatever its sending setting); every trading button disabled with it (as E2).
- Step 3: "The exchange needs Namehold's local node, or your own hsd on this computer, able to send." (SPV cannot send).
- Step 4: no banner; buttons enabled.
- Passes if only a node on this computer that can send lets the exchange act.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E3-1-remote`, `E3-2-remote-no-sending`.
- **Cleanup / end state:** Connections as noted.

## E4 — Not enabled yet, and enabling

- **Priority:** P1
- **Covers:** spec §9 `exchange_enable`, R14 (no key stored), R17 (order of the gates)
- **Preconditions:** a regtest recovery-phrase profile on which the exchange was never enabled (the Seller during [exchange-setup](../../procedures/exchange-setup.md) step 8, or a throwaway profile); its signer locked (switch to it: switching locks it, [switch-profile](../../procedures/switch-profile.md)). On a fresh setup no relay is set yet; otherwise note which settings are set.

Steps:

1. Open Exchange. Read the banners, the header buttons, the BTC account card, the Order book tab. Hover the disabled buttons.
2. Press `Enable exchange` with the signer locked.
3. Unlock (readiness pill › Unlock, passphrase in the secure window). Press `Enable exchange`.
4. Read the page again; `Refresh`. Check both mempools: `H getrawmempool`, `B getrawmempool`.

Expected:

- Step 1: no amber banner; the blue banner as in E1 with `Enable exchange` enabled. `Refresh`, `Make an offer…` and `Withdraw…` disabled with "Enable the exchange first." (also `Make an offer…` with no relay: not enabled comes first). `Settings` is enabled. BTC account: "Enable the exchange to get a Bitcoin account made from this wallet's recovery phrase." With no relay set, the Order book tab reads "No relay is set. Set one in Exchange settings to see and publish offers."
- Step 2: a red toast "Your signer is locked — click Unlock first."; the banner stays. No secure window asks for the passphrase on its own.
- Step 3: toast "Exchange enabled"; the blue banner is gone. Only the passphrase window opened; no transaction window.
- Step 4: `Refresh` and `Withdraw…` enabled. With no relay set, `Make an offer…` is disabled with "No relay is set. Set one in Exchange settings to see and publish offers." With no Bitcoin source set (fresh setup), the BTC account card shows `Confirmed`, `Unconfirmed`, `Locked in trades` at 0 and the red line "Invalid input: no Bitcoin source is set: add one in Exchange settings", and a `Receive address` `bcrt1q…`. Both mempools unchanged (enabling makes no transaction).
- Passes if the gates come in that order and enabling needs the unlocked signer and nothing else.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E4-1-not-enabled`, `E4-3-enabled`.
- **Cleanup / end state:** the profile enabled.

## E5 — No meeting point set

- **Priority:** P2
- **Covers:** spec §9 (`exchange_list_offers`, `exchange_create_offer`), `NO_RELAY`
- **Preconditions:** the Buyer, enabled, settings as in setup; **no trade open on either profile** (Commits and lock announcements go through the relay); OB live if [E11](offers-and-taking.md#e11--publish-a-buy-offer) ran.

Steps:

1. Exchange › `Settings` → empty `Your own relay (optional)` (regtest has no default Nostr relays, so nothing is left) → `Save`. Read the secure window; `Confirm & Sign`.
2. Read the Order book tab, `Make an offer…`, My offers' `Withdraw`, `Refresh`.
3. `Settings` → `Your own relay (optional)` `http://127.0.0.1:8787` → `Save` → confirm.
4. After 2 minutes reopen My offers.

Expected:

- Step 1: secure window `Change exchange settings` with the one row `Relay` "http://127.0.0.1:8787 → none". Toast "Exchange settings saved".
- Step 2: the Order book tab reads "No meeting point is set. Name Nostr relays or a relay in Exchange settings to see and publish offers." (no offer tables); `Make an offer…` and every `Withdraw` disabled with that sentence; `Refresh` stays enabled.
- Step 3: the row `Relay` "none → http://127.0.0.1:8787"; the order book is back within 15 s.
- Step 4: OB reads `Live` again (record whether it went `Paused` meanwhile: its heartbeat had no relay).
- Passes if an empty relay is a clear, named state and setting it back restores the order book.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshot `E5-2-no-relay`.
- **Cleanup / end state:** the relay URL set again.

## E6 — The Exchange settings dialog and its secure window

- **Priority:** P1
- **Covers:** spec §9 `exchange_update_settings`, SECURITY.md "The exchange" › Settings
- **Regression of:** exchange QA finding 17 (a confirmed save stored the settings and then froze the whole app: the dialog stayed on "Confirm in the secure window…" and every later command hung)
- **Preconditions:** an enabled profile (fresh setup: nothing set yet; otherwise every "none →" below reads the current value instead, and step 3's valid save changes nothing: then do it with one value changed and back). No trade open (step 5 changes the login for a moment only in a declined window).

Steps:

1. Exchange › `Settings`. Read the dialog.
2. Field checks (no secure window yet): type `https://relay.example` in `Nostr relays, one per line`; then `ftp://x` in `Your own relay (optional)`; then `127.0.0.1:18443` as the only source line; then `namehold` in `bitcoind RPC login (user:password)`. Clear each after reading it.
3. Backend checks, with the source `bitcoind+http://127.0.0.1:18443` and the login `namehold:regtest` filled in, and in turn: relay `http://192.0.2.1:8787`; watchtower `http://192.0.2.1:8787`; source `bitcoind+http://192.0.2.1:18443`; relay `http://u:p@127.0.0.1:8787`; six source lines (`bitcoind+http://127.0.0.1:18443` six times). `Save` after each, then restore the field.
4. The valid values of [exchange-setup](../../procedures/exchange-setup.md) step 9 → `Save` → read the secure window → `Cancel`. `Save` again → `Confirm & Sign`.
5. Reopen `Settings`. Read the login field. `Save` with nothing changed. Reopen; type `namehold:other` in the login → `Save` → read → `Cancel`; `Cancel` the dialog. Reopen; press `Clear` → read → `Save` → read → `Cancel`; press `Keep`; `Cancel` the dialog.
6. `Refresh` on the page; read the BTC account card.

Expected:

- Step 1: title `Exchange settings`; fields `Nostr relays, one per line` (empty, placeholder: the network's defaults, none on regtest) with the hint "Offers and trade messages go through these public Nostr relays, which other people run; messages are encrypted to the other side. Leave empty to use the defaults.", `Your own relay (optional)` (placeholder "Empty: the Nostr relays above", hint "Only if you trust someone who runs namehold-relay: when set, it is used instead of the Nostr relays."), `Watchtower URL (optional)` (placeholder "Empty: no watchtower"), `Bitcoin sources, one per line` (placeholder `https://mempool.space/api` / `bitcoind+http://127.0.0.1:18443`) with the hint "Bitcoin sources are not trusted: Namehold checks every lock by proof of work and needs every source to agree. An Esplora base URL looks like https://mempool.space/api; a local Bitcoin Core is bitcoind+http://127.0.0.1:18443.", `bitcoind RPC login (user:password)` (a password field); `Cancel` and `Save`. With no source: "Enter at least one Bitcoin source" under the sources and `Save` disabled. No `Clear` button while no login is stored.
- Step 2: "\"https://relay.example\" is not a Nostr relay: use wss://host"; "Enter a URL that starts with http:// or https://"; "\"127.0.0.1:18443\" is not a Bitcoin source: use an Esplora URL (https://…) or bitcoind+http://host:port"; "Enter the RPC login as user:password". `Save` disabled while one shows.
- Step 3: no secure window opens; under the fields, in red: "Invalid input: the relay URL must be https, or http to this machine"; "Invalid input: the watchtower URL must be https, or http to this machine"; "Invalid input: the Bitcoin source must be https, or http to this machine"; "Invalid input: the relay URL must not carry a user name or password"; "Invalid input: at most 5 Bitcoin sources". Nothing is contacted (192.0.2.1 is a documentation address).
- Step 4: secure window `Change exchange settings`, message "Bitcoin sources are not trusted: Namehold checks every lock by proof of work and needs every source to agree. The Nostr relays, the relay and the watchtower hold no key and no secret.", rows `Relay` "none → http://127.0.0.1:8787", `Watchtower` "none → http://127.0.0.1:8787", `Bitcoin sources` "none → bitcoind+http://127.0.0.1:18443", `bitcoind login` "set". The login's value appears nowhere in the window. `Cancel` → the dialog shows "Confirmation declined" in red and stays open. After `Confirm & Sign`: toast "Exchange settings saved", the dialog closes, and the app still answers: `Refresh` updates `Checked` and the order book stops reading "No relay is set. Set one in Exchange settings to see and publish offers." (a dialog stuck on "Confirm in the secure window…" with the settings already stored is finding 17 again).
- Step 5: the login field is empty with the placeholder `set`, under it "A user:password is set. Leave the field empty to keep it.", and a `Clear` button. `Save` with nothing changed: no secure window, toast "Exchange settings saved". `namehold:other`: one row `bitcoind login` "changed" (never the value). `Clear`: the button reads `Keep`, the field is disabled and empty, the line reads "The stored user:password is removed when you save."; its window has one row `bitcoind login` "cleared". Every declined window leaves the settings as they were.
- Step 6: the card shows balances and no red source error (the stored login still works).
- Passes if every change is shown old → new in the secure window, the login only as set / changed / cleared, and nothing unconfirmed is stored.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E6-1-dialog`, `E6-3-refused`, `E6-4-secure-window`, `E6-5-changed`, `E6-5-cleared`.
- **Cleanup / end state:** the regtest settings of the setup stored.

## E7 — The bitcoind login never reaches the screen, the renderer or a log

- **Priority:** P2
- **Covers:** SECURITY.md "The exchange" › Settings ("the bitcoind RPC login is a sensitive setting, redacted from `get_settings`"), `exchange_io/env.rs`, `commands/settings.rs::redact`
- **Regression of:** exchange QA finding 3 (`get_settings` returned the bitcoind login to the renderer)
- **Preconditions:** after E6; mode B for step 2 (mode A: step 2 BLOCKED ✗); no trade between its Commits (step 3 breaks the Bitcoin source for a minute).

Steps:

1. Recall E6's windows: the login showed only as `set`, `changed`, `cleared`.
2. Inspector console: `const s = await window.__TAURI_INTERNALS__.invoke("get_settings"); Object.keys(s).filter(k => k.includes("exchange"))`, then `s["exchange_bitcoind_rpc_auth.regtest"]`.
3. Exchange › `Settings` → login `namehold:wrong` → `Save` → confirm (row `bitcoind login` "changed"). `Refresh`; read the BTC account card's red line. Then set `namehold:regtest` back the same way and `Refresh`.
4. `grep -c "namehold:regtest\|namehold:wrong" "$QA_DIR/tauri-dev.log" "$QA_DIR/relay/relay.log"`.

Expected:

- Step 2: the keys `exchange_relay_url.regtest`, `exchange_watchtower_url.regtest`, `exchange_btc_sources.regtest` are there with the values set; `exchange_bitcoind_rpc_auth.regtest` is absent, so the second expression gives `undefined`, and in its place the marker `__has_exchange_bitcoind_rpc_auth.regtest` reads `"true"`. A value under any key holding the login is ❌: record the key's name, never the value.
- Step 3: the card's red line reads "Bitcoin source http://127.0.0.1:18443/: the RPC login was refused: check the bitcoind login in Exchange settings": no credentials (no `wrong`, no `regtest`), and no "Node RPC error:" prefix. After restoring, no red line.
- Step 4: both counts 0.
- Passes if the login appears in no window, no renderer answer and no log.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** the inspector output (key names only); screenshot `E7-3-wrong-login`; the grep counts.
- **Cleanup / end state:** the login `namehold:regtest` stored; the card reads without error.
