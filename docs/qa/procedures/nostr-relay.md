# nostr-relay

Local Nostr relays for the exchange cases that run over Nostr ([scenarios/exchange/nostr.md](../scenarios/exchange/nostr.md)), and the commands to read what the app put on them. The app talks to these relays exactly as to public ones ([spec 2026-11-12](../../specs/2026-11-12-exchange-nostr-transport.md)); on regtest there are no default relays, so a case names its own.

- **Parameters:** how many relays (1 to 3).
- **Blocks mined:** none.
- **Needs:** Docker running (Docker Desktop on macOS), and on first use the image `ghcr.io/hoytech/strfry` (229 MB, pinned by digest in the script). Node 22 or newer for the reader.
- **Checked:** 2026-11-12 on macOS with Docker 29.9: three relays started, and `live_exchange_nostr` against them took, served, replaced and deleted offers and took a burst of 20 writes on each.

## Steps

1. **Start the relays.** `bash "$REPO/scripts/nostr-relay.sh" start 3` starts relays on `ws://127.0.0.1:7777`, `:7778` and `:7779` (one relay: `start` alone). Each prints `relay up: ws://127.0.0.1:<port>`. `bash "$REPO/scripts/nostr-relay.sh" status` lists them; `urls` prints the lines to paste into Exchange settings.
2. **Point the app at them.** Exchange › `Settings` → `Nostr relays, one per line`: the URLs of step 1; `Your own relay (optional)` empty (otherwise the custom relay is used instead of Nostr) → `Save` → in the secure window the row `Nostr relays` "the defaults → ws://127.0.0.1:7777, ws://127.0.0.1:7778, ws://127.0.0.1:7779" → `Confirm & Sign`. The exchange settings are per network for the whole app, not per profile: this sets both the Buyer and the Seller.
3. **Read a relay** at any time (read-only: the reader sends no event):

   ```bash
   NQ() { node "$REPO/scripts/nostr-req.mjs" "${2:-ws://127.0.0.1:7777}" "$1" ${3:-}; }
   NQ '{"kinds":[37491]}'                        # every offer event (kind 37491)
   NQ '{"kinds":[37491],"#d":["<offer id>"]}'    # one offer's address
   NQ '{"kinds":[7491],"#p":["<x-only key>"]}'   # trade messages to one key (kind 7491)
   NQ '{"kinds":[5]}'                            # deletions
   NQ '{"kinds":[7491]}' ws://127.0.0.1:7778 --full   # another relay, whole contents
   ```

   Each line is `kind created_at author tags "content"` (the author in full, 64 hex); the last line counts the events.

## Telling whose event it is

- An **offer id** is the `d` tag of its event; the app shows it as `data-offer-id` on order-book rows, and the run's own database has it: `sqlite3 "$QA_DIR/qa-home/.namehold/portfolio.db" "select id, side, state, last_published_at, last_error from exchange_offers"`.
- An offer's **maker key** is `makerRelayPubkey` inside `signed_json` of that table (66 hex). The event's author, and the `p` tag of a message to that maker, is the same key **without its first two hex characters** (the x coordinate, 64 hex).
- Never copy `relay_key_hex` (a secret key) into a report.

## End state

Relays running until the case says otherwise. `bash "$REPO/scripts/nostr-relay.sh" stop [port]` stops one or all (their events stay and come back with `start`); `reset` stops all and deletes their data. Put the app back with Exchange › `Settings` → `Nostr relays` empty → `Save` → confirm.
