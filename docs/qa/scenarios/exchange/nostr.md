---
area: HNS ⇄ BTC exchange over Nostr — the Nostr relays in Exchange settings, an offer's events (publish, renew, withdraw), encrypted trade messages, each message handled once across restarts, relays going down (the 2-relay quorum), a custom relay in place of Nostr, the public default relays
priority: P1–P3
blocks: HNS 2, BTC 2
requires:
  - procedures/exchange-setup.md
  - procedures/nostr-relay.md
  - procedures/switch-profile.md
last_verified: "not run yet (written 2026-11-12; the stand of procedures/nostr-relay.md was checked that day)"
---

# The exchange over Nostr

By default the exchange's offers and trade messages travel as events on public Nostr relays ([spec 2026-11-12](../../../specs/2026-11-12-exchange-nostr-transport.md)): an offer is an addressable event of kind **37491** whose `d` tag is the offer id, renewed by the heartbeat and replaced by a withdrawal; a trade message is an event of kind **7491**, NIP-44-encrypted to its recipient, with a `p` tag naming it. A publish counts when two relays took the event (one when only one is set); a read when one answered. A custom relay (`namehold-relay`) set in Exchange settings is used instead of Nostr.

These cases run on regtest against local relays started by [nostr-relay](../../procedures/nostr-relay.md), except E47, which writes to public relays. [exchange-setup](../../procedures/exchange-setup.md) sets a custom relay (`http://127.0.0.1:8787`); every case below except E46 starts by clearing it, so offers go over Nostr.

Shell helpers: `NQ` from [nostr-relay](../../procedures/nostr-relay.md#steps), `RELAY` and the rest from [exchange-setup](../../procedures/exchange-setup.md#variables-and-helpers), and:

```bash
DB="$QA_DIR/qa-home/.namehold/portfolio.db"   # the run's own app database (mode B)
maker_x() { sqlite3 "$DB" "select signed_json from exchange_offers where id = '$1'" | python3 -c 'import json,sys; print(json.load(sys.stdin)["offer"]["makerRelayPubkey"][2:])'; }
```

## E41 — Nostr relays in Exchange settings

- **Priority:** P1
- **Covers:** spec 2026-11-12 R9, R10, R11; `env::with_nostr`, `settings_rows`, `NO_RELAY`
- **Preconditions:** [exchange-setup](../../procedures/exchange-setup.md) done; [nostr-relay](../../procedures/nostr-relay.md) step 1 with 3 relays; the Seller active; no trade open.

Steps:

1. Exchange › `Settings`. Read the dialog's first two fields.
2. Field check (no window yet): type `https://relay.example` in `Nostr relays, one per line`. Clear it.
3. Backend checks: in turn `ws://192.0.2.1:7777`, `wss://u:p@127.0.0.1:7777`, and eleven lines `ws://127.0.0.1:7777` … `ws://127.0.0.1:7787`; `Save` after each, then clear the field.
4. `Nostr relays`: the three URLs of `bash "$REPO/scripts/nostr-relay.sh" urls`; `Your own relay (optional)` emptied → `Save`. Read the secure window → `Confirm & Sign`.
5. Reopen `Settings`; read both fields. `Cancel`.
6. `Settings` → empty `Nostr relays` → `Save` → read → `Confirm & Sign`. Read the Order book tab and `Make an offer…`.
7. Put the three URLs back as in step 4.

Expected:

- Step 1: `Nostr relays, one per line`, empty, its placeholder empty (regtest has no default relays; on mainnet it lists the six defaults), and under it "Offers and trade messages go through these public Nostr relays, which other people run; messages are encrypted to the other side. Leave empty to use the defaults."; then `Your own relay (optional)` holding `http://127.0.0.1:8787`, placeholder "Empty: the Nostr relays above", and "Only if you trust someone who runs namehold-relay: when set, it is used instead of the Nostr relays."
- Step 2: in red "\"https://relay.example\" is not a Nostr relay: use wss://host"; `Save` disabled.
- Step 3: no secure window; in red, in turn: "Invalid input: the Nostr relay ws://192.0.2.1:7777 must be wss://, or ws:// to this machine"; "Invalid input: the Nostr relay wss://u:p@127.0.0.1:7777 must not carry a user name or password"; "Invalid input: at most 10 Nostr relays".
- Step 4: secure window `Change exchange settings`, message ending "The Nostr relays, the relay and the watchtower hold no key and no secret.", rows `Relay` "http://127.0.0.1:8787 → none" and `Nostr relays` "the defaults → ws://127.0.0.1:7777, ws://127.0.0.1:7778, ws://127.0.0.1:7779"; toast "Exchange settings saved".
- Step 5: the three URLs one per line; the custom relay empty.
- Step 6: row `Nostr relays` "ws://127.0.0.1:7777, ws://127.0.0.1:7778, ws://127.0.0.1:7779 → the defaults"; the Order book tab reads "No meeting point is set. Name Nostr relays or a relay in Exchange settings to see and publish offers." and `Make an offer…` is disabled with that sentence (regtest: no defaults, no relay).
- Passes if only `wss://` (or `ws://` to this machine) relays without logins are taken, every change is confirmed, and an empty list on regtest is a named state.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E41-1-dialog`, `E41-3-refused`, `E41-4-secure-window`, `E41-6-no-meeting-point`.
- **Cleanup / end state:** the three local relays set, the custom relay empty.

## E42 — An offer's life on Nostr

- **Priority:** P1
- **Covers:** R3, R4, R5; `NostrMeetingPoint::{publish_offer, offers, withdraw_offer}`, the job's heartbeat
- **Preconditions:** E41's end state; the Seller active; the daemon running.

Steps:

1. `Make an offer…` → `Sell HNS`, `10000`, `1`, `20` → `Publish offer`. Read the secure window → `Confirm & Sign`. Note the offer id: `sqlite3 "$DB" "select id from exchange_offers where state = 'live' order by created_at desc limit 1"`.
2. On each relay: `NQ '{"kinds":[37491],"#d":["<id>"]}'`, then with `ws://127.0.0.1:7778` and `:7779`.
3. Compare the event's author with `maker_x <id>`.
4. [switch-profile](../../procedures/switch-profile.md) to the Buyer; Exchange › Order book; wait up to 15 s.
5. Wait 3 minutes. Read the event on `:7777` again (`created_at`), and the count of versions.
6. Back to the Seller: My offers → `Withdraw` on the offer. On `:7777`: `NQ '{"kinds":[37491],"#d":["<id>"]}'` and `NQ '{"kinds":[5]}'`.
7. On the Buyer: the order book after 15 s.

Expected:

- Step 1: secure window `Publish an offer` with the row `Published on` "Nostr relays ws://127.0.0.1:7777, ws://127.0.0.1:7778, ws://127.0.0.1:7779"; toast "Offer published"; the row in My offers reads `Live`.
- Step 2: on every relay exactly 1 event: kind `37491`, tags `["d","<id>"]`, `["n","regtest"]` and `["expiration","<created_at + 300>"]`, content starting `{"offer":{"version":1,"network":"regtest","side":"sell"`.
- Step 3: the event's author equals `maker_x <id>`.
- Step 4: the offer is in the Buyer's `Sell HNS` table with `Take`, price `10,000`.
- Step 5: still 1 event per relay (each renewal replaces the last), `created_at` later than in step 2 by about a minute or more; My offers still `Live`.
- Step 6: toast "Offer withdrawn"; the row leaves My offers; `0 events` for the address (strfry honours the deletion); one kind-5 event with tags `["a","37491:<author>:<id>"]` and `["k","37491"]`.
- Step 7: the offer is gone from the order book.
- Passes if the offer is one event per relay, signed by the offer's own key, renewed in place, and gone after the withdrawal.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E42-1-secure-window`, `E42-4-order-book`, `E42-7-gone`; the `NQ` outputs of steps 2, 5 and 6.
- **Cleanup / end state:** no offer of this case live.

## E43 — Trade messages are encrypted and addressed

- **Priority:** P1
- **Covers:** R6, R7, R12; `NostrMeetingPoint::{send, inbox}`, NIP-44 v2
- **Preconditions:** E41's end state; the Seller has a live sell offer over Nostr (E42 step 1, call it ON); the Buyer unlocked.

Steps:

1. On the Buyer: Order book → `Take` on ON → `1` HNS → take it (as [E14](offers-and-taking.md#e14--the-take-dialog-amount-checks-and-the-exact-btc)). Wait until the card reads "Confirm to start the trade" (`Refresh`).
2. On `:7777`: `NQ '{"kinds":[7491],"#p":["'"$(maker_x <ON id>)"'"]}'`. Note the event's author: the taker's key, `T`.
3. `NQ '{"kinds":[7491],"authors":["'"$(maker_x <ON id>)"'"]}'`: the maker's reply.
4. Search every 7491 content for readable text: `NQ '{"kinds":[7491]}' | grep -c -E 'regtest|terms|hns'`.
5. `Cancel` the trade on the Buyer.

Expected:

- Step 1: toast "Trade started · it is under My trades"; the card reaches "Confirm to start the trade".
- Step 2: one event (the take): kind `7491`, tags `["p","<maker_x>"]`, `["n","regtest"]`, `["expiration","<created_at + 604800>"]`; its content is base64 (letters, digits, `+`, `/`, `=`), never JSON.
- Step 3: one event from the maker (the accept) with `["p","T"]`, also base64.
- Step 4: `0`: no message body is readable on the relay.
- Step 5: "Trade cancelled"; one more 7491 event from the taker (the cancel's decline) to the maker.
- Passes if every trade message is on the relay only encrypted, from its sender's key, addressed by `p` to the other side's key.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** the `NQ` outputs of steps 2–4; screenshot `E43-1-card`.
- **Cleanup / end state:** ON live, no trade open.

## E44 — Each message is handled once, a restart included

- **Priority:** P2
- **Covers:** R7 (the cursor with the ids handled); regression of the 2026-11-12 review finding "a re-read declined take spent another pooled hash each time"
- **Preconditions:** E41's end state; the Seller has a live sell offer over Nostr with no open trade (ON); the Buyer unlocked; the daemon running.

Steps:

1. As [E16](offers-and-taking.md#e16--at-most-three-open-trades-per-offer) step 1: the Buyer takes ON four times with `1` HNS, within two minutes, confirming none. Wait 3 minutes; `Refresh`; find the declined card.
2. `NQ '{"kinds":[7491],"authors":["<maker_x ON>"]}'` on `:7777`, count the events. Read the maker's inbox cursor: `sqlite3 "$DB" "select substr(cursor,1,90) from exchange_inbox where relay_pubkey = (select json_extract(signed_json,'$.offer.makerRelayPubkey') from exchange_offers where id = '<ON id>')"`.
3. Quit the app and stop the daemon; start them again ([environment](../../environment.md), mode B). Wait 5 minutes (the inbox reads back 15 minutes, so step 1's messages are read again).
4. Count the maker's events again as in step 2. Read the four cards.
5. `Cancel` the three open cards.

Expected:

- Step 1: three cards "Confirm to start the trade", one "Trade expired" with "The other side declined: the maker has too many trades open on this offer".
- Step 2: 4 events from the maker (three accepts, one decline); the cursor starts `nostr:<unix time>:` followed by event ids separated by commas.
- Step 4: still 4 events from the maker: nothing was answered twice. The cards are as in step 1.
- Passes if no message is answered again after a restart within the read-back window.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** the counts and the cursor of steps 2 and 4; screenshot `E44-4-cards`.
- **Cleanup / end state:** no open trade on ON.

## E45 — Relays going down: the quorum

- **Priority:** P2
- **Covers:** R8; `NostrClient::publish` (`min(2, relays)` must take an event), the offer's `Paused` state and its recovery
- **Preconditions:** E41's end state with 3 relays; the Seller active, no offer of its own live.

Steps:

1. `bash "$REPO/scripts/nostr-relay.sh" stop 7779`. Publish a sell offer (`10000`, `1`, `20`) → `Confirm & Sign`.
2. `bash "$REPO/scripts/nostr-relay.sh" stop 7778`. Publish another (`11000`, `1`, `20`) → `Confirm & Sign`. Read the dialog.
3. Close the dialog; read My offers.
4. `bash "$REPO/scripts/nostr-relay.sh" start 3`. Wait 3 minutes; reopen My offers; `NQ '{"kinds":[37491]}' ws://127.0.0.1:7778`.

Expected:

- Step 1: "Offer published"; `Live` (two relays of three took it).
- Step 2: no toast; in red under the dialog a sentence containing "the offer is saved but the relay refused it: Nostr: 1 of 3 relays took the event (2 needed)", then "ws://127.0.0.1:7778: refused the connection (is it running?)" and the same for `:7779`. Never the generic "Couldn't reach the configured endpoint".
- Step 3: both offers in My offers: the first `Live`, the second `Paused` (hover: "Not renewed on the relay right now, so others do not see it").
- Step 4: both `Live`; `:7778` now holds both offers (the heartbeat republished them on all three).
- Passes if one relay down changes nothing, two down are named in the refusal, and the offers come back on their own.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E45-2-refused`, `E45-3-paused`, `E45-4-live`.
- **Cleanup / end state:** three relays running; both offers withdrawn.

## E46 — A custom relay replaces Nostr

- **Priority:** P2
- **Covers:** R11 (`env::meeting_point`): the custom relay is used instead of Nostr when set
- **Preconditions:** E41's end state; `namehold-relay` of setup running; the Seller active.

Steps:

1. `Settings` → `Your own relay (optional)` `http://127.0.0.1:8787` (the Nostr relays stay) → `Save` → confirm.
2. Publish a sell offer (`12000`, `1`, `20`) → read the secure window → `Confirm & Sign`. Note its id.
3. `curl -s "$RELAY/v1/offers?network=regtest"`; `NQ '{"kinds":[37491],"#d":["<id>"]}'`.
4. Withdraw it. `Settings` → empty the custom relay → `Save` → confirm. Publish another offer (`12000`, `1`, `20`) and repeat step 3 with its id.

Expected:

- Step 1: row `Relay` "none → http://127.0.0.1:8787".
- Step 2: row `Published on` "http://127.0.0.1:8787".
- Step 3: the relay's list holds the offer; `0 events` on the Nostr relay.
- Step 4: the window says `Published on` "Nostr relays …"; the second offer is on the Nostr relay (1 event) and not in the relay's list.
- Passes if exactly one meeting point is used at a time, and the window says which.

- **Blocks mined:** HNS 0, BTC 0
- **Evidence:** screenshots `E46-2-relay`, `E46-4-nostr`; the outputs of step 3.
- **Cleanup / end state:** the custom relay empty; the offers withdrawn.

## E40 — The happy path over Nostr

- **Priority:** P1
- **Covers:** spec 2026-11-12 (R3–R7, R11): a whole trade with the custom relay empty
- **Preconditions:** E41's end state; the Buyer funded as in setup; the daemon running.

Steps:

1. The Seller publishes a sell offer (as E10, over Nostr). On the Buyer, the order book.
2. Run [E17](trades.md#e17--the-happy-path-a-buyer-takes-a-sell-offer)'s steps 2 onwards on this offer.
3. During the trade: `grep -c "POST /v1" "$QA_DIR/relay/relay.log"` before and after; `NQ '{"kinds":[7491]}'` on `:7777`.

Expected:

- Step 1: the offer shows in the Buyer's order book within 15 s.
- Step 2: as E17's expectations.
- Step 3: the relay log's `POST /v1` count does not grow for offers or messages (the watchtower's `POST /v1/watch` still goes there if it is set); the trade's messages (take, accept, the lock announcements) are 7491 events, all base64.
- Passes if a trade completes with the custom relay empty.

- **Blocks mined:** HNS 2, BTC 2
- **Evidence:** screenshots `E40-1-order-book`, `E40-complete`; the outputs of step 3.
- **Cleanup / end state:** the trade complete.

## E47 — The public default relays before a release

- **Priority:** P2 (once per release)
- **Covers:** R10 (the defaults), spec §5 "Public relays differ, and change"
- **Preconditions:** internet access. **Only with the maintainer's agreement for this run:** it publishes short-lived test events to public relays, outside regtest ([rules](../../rules.md)). Each relay gets about 25 events from a throwaway key, with a random network tag, contents that name nobody, and a 10-minute expiration.

Steps:

1. `cd "$REPO/src-tauri" && HNS_IT_NOSTR=1 cargo test --lib live_exchange_nostr -- --nocapture --test-threads=1`.

Expected:

- One line per default relay (`env::DEFAULT_NOSTR_RELAYS`): `offer taken true served true | replaced true | … | message taken true served true | burst N/20`, and `test result: ok` (at least 2 relays take and serve both kinds).
- Passes if every default relay takes and serves both kinds. A relay that fails (as relay.damus.io, nos.lol and nostr.mom did on 2026-11-12) is reported with its line, to be replaced in the defaults.

- **Blocks mined:** none
- **Evidence:** the test's output.
- **Cleanup / end state:** nothing to clean: the events expire on their own.
