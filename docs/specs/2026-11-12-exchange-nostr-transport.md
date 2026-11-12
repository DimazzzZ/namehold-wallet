# Exchange offers and messages over Nostr

Status: accepted 2026-11-12, being implemented on `feat/exchange-nostr`. Extends [2026-11-07 HNS ⇄ BTC exchange](./2026-11-07-hns-btc-exchange.md) (§8 relay API, §9 commands); the trade protocol, its signatures and every on-chain check stay as that spec says.

## 1. Summary

The exchange needs a place where offers are listed and where the two parties of a trade exchange their messages. Until now that place was a `namehold-relay` server that somebody had to run, and nobody runs a public one, so a fresh install could not trade. With this change the default place is **Nostr**: offers and messages travel as signed events on several public Nostr relays that independent people run, so the exchange works out of the box and the Namehold project operates no server for it. A `namehold-relay` that someone does run can still be entered in Exchange settings and is then used instead; the watchtower stays a `namehold-relay` feature a user opts into. Trade messages are encrypted to their recipient (NIP-44 v2); offers are public, as an order book has to be.

## 2. Terms

- **Meeting point**: where offers are listed and messages delivered. Either the **Nostr relays** of the network's settings (the default) or a **custom relay**, a `namehold-relay` URL the user entered.
- **Nostr relay**: a public server speaking NIP-01 over a WebSocket (`wss://`). Not operated by the project.
- **Offer event**: kind **37491**, addressable (NIP-01: kinds 30000–39999 keep the latest version per author and `d` tag). Its `content` is the `SignedOffer` JSON of `relay_msg`, unchanged.
- **Withdrawal event**: a version of the offer event whose `content` is `{"withdrawn":"<offer id>","maker":"<maker key>","signature":"<withdraw signature>"}`, published together with a NIP-09 deletion (kind 5) of the offer's address.
- **Message event**: kind **7491**, regular (stored). Its `content` is the NIP-44 v2 encryption of the `Envelope` JSON of `relay_msg` to the recipient.
- **Relay key**: the random secp256k1 key `relay_msg` already gives each offer and trade. It signs the inner `SignedOffer` / `Envelope` with ECDSA as before, and the Nostr event with BIP-340 Schnorr. The event's `pubkey` is the relay key's x coordinate (the compressed key without its first byte).
- **Accepted**: a Nostr relay answered `["OK", <id>, true, …]` for an event.

## 3. Requirements

| # | Requirement | Code | Test |
|---|---|---|---|
| R1 | An event's id is SHA-256 of the NIP-01 serialisation `[0,pubkey,created_at,kind,tags,content]`; it is signed with BIP-340 over that id by the relay key. An event read from a relay counts only if its id recomputes and its signature verifies; otherwise it is dropped. | `exchange_io/nostr/event.rs` | `exchange_nostr_tests::event_*` |
| R2 | NIP-44 v2 encryption and decryption match the official test vectors (conversation key, message keys, padding, encryption, and every listed invalid case refused). Plaintexts are 1–65535 bytes behind the u16 length prefix, as in the vectors; the NIP's later 6-byte extended prefix (65536 bytes and more) is refused both ways. | `exchange_io/nostr/nip44.rs` | `exchange_nostr_tests::nip44::*` |
| R3 | An offer is published as kind 37491 with tags `["d", <offer id>]`, `["n", <network>]` and `["expiration", now + 300]`; the `content` is the `SignedOffer`. Each heartbeat publishes a new version with a fresh `created_at`. | `exchange_io/nostr/transport.rs` | `exchange_nostr_tests::transport::offer_*` |
| R4 | Listing reads kind 37491 with `#n` = network and `since` = now − 180 from every Nostr relay, keeps per (author, `d`) only the newest version (lowest id on a tie), and lists an offer only if: its `SignedOffer` verifies and passes `check_shape`, its id equals `d`, the event's author equals the x coordinate of `makerRelayPubkey`, and the newest version is not a withdrawal. | `transport.rs` | `exchange_nostr_tests::transport::listing_*` |
| R5 | Withdrawing publishes the withdrawal event and a kind 5 deletion with `["a", "37491:<author>:<offer id>"]`. Only the maker's key can publish a newer version of its address (R4's author rule), so a newest version that is a withdrawal hides the offer, on relays that ignore the deletion too; it is not counted as a bad offer. | `transport.rs` | `exchange_nostr_tests::transport::withdrawal_*` |
| R6 | A message is published as kind 7491 with tags `["p", <recipient x-only>]`, `["n", <network>]` and `["expiration", now + 7 days]`; its `content` is the NIP-44 v2 encryption of the `Envelope` with the conversation key of the sender's relay key and the recipient's key. | `transport.rs` | `exchange_nostr_tests::transport::message_*` |
| R7 | The inbox reads kind 7491 with `#p` = the inbox key and `since` = cursor time − 900 (a week back without one) from every Nostr relay, decrypts each event with the inbox key and the event's author, and keeps a message only if `Envelope::parse_for(to, network)` accepts it and its `from` key's x coordinate equals the event's author. Each event is handed to the job once: the stored cursor is `nostr:<time>:<id>,…`, the time of the newest message handed over (never an unchecked event's, never back) and the ids of every event in the window the next read covers (at most 500), so a re-read, after a restart too, skips them. A cursor without the prefix is a `namehold-relay`'s and is not used. | `transport.rs` | `exchange_nostr_tests::transport::inbox_*` |
| R8 | A publish succeeds when at least `min(2, number of Nostr relays)` relays accepted the event; otherwise it fails with the relays' answers. Reads ask every relay in parallel and succeed when at least one answered with `EOSE`; each relay has a connect timeout of 10 s and an answer timeout of 15 s. An event read counts only if it matches the read's own filter and verifies (R1); a frame is at most 128 KiB and one relay's answer to one read at most 4 MiB. | `exchange_io/nostr/client.rs` | `exchange_nostr_tests::client::{quorum_*, query_*}` |
| R9 | A Nostr relay URL must be `wss://` (or `ws://` to this machine, for tests), plain ASCII, with no user name or password. The WebSocket client uses the project's rustls with the ring provider and follows no HTTP redirect. | `env.rs`, `client.rs` | `exchange_cmd_tests::nostr_settings::settings_*` |
| R10 | Each network has a list of Nostr relays in Exchange settings. Empty means the defaults: `wss://relay.primal.net`, `wss://offchain.pub`, `wss://nostr.bitcoiner.social`, `wss://relay.nostr.net`, `wss://nostr.oxtr.dev`, `wss://relay.mostro.network` on mainnet and testnet (the relays the live check of 2026-11-12 found taking and serving both kinds); none on regtest and simnet (a test names its own). A change is confirmed in the secure window like the other exchange settings. | `env.rs`, `commands/exchange.rs` | `exchange_cmd_tests::nostr_settings::{the_defaults_*, a_nostr_change_*}` |
| R11 | The meeting point is the custom relay when one is set, otherwise the Nostr relays. The job and the commands reach either only through one trait (`MeetingPoint`); the watchtower stays a separate `namehold-relay` client. | `exchange_io/meeting.rs`, `job.rs`, `env.rs` | `exchange_cmd_tests::nostr_settings::meeting_point_*`; the job's fake checks every write carries the key that signed it (`exchange_fake_btc`) |
| R12 | No secret leaves the app through Nostr: offer and message contents are what `namehold-relay` received before (signed offers, signed envelopes with public terms), messages encrypted; the relay key's secret signs only. | `transport.rs` | `exchange_layering_tests` |

## 4. Explicitly not enforced

- **Delivery.** Nostr relays may drop, delay or refuse events, and may keep them after their `expiration`. A missed message is not resent by the relay; the job's own repeats (offer heartbeats, lock announcements every 300 s) are the only retry.
- **Who talks to whom.** Event authors, recipients (`p` tags) and timestamps are public; only the message bodies are encrypted. Relay keys are fresh per offer and trade, so they do not name the user, but the user's IP address is visible to every relay the app connects to.
- **Censorship resistance against all relays at once.** If every configured relay refuses our kinds, the exchange cannot list or trade until the user enters other relays.
- **NIP-44 limits** as stated by NIP-44 itself: no forward secrecy, no deniability.

## 5. Known gaps

- **Kind numbers 37491 and 7491** were free in the NIPs kind table on 2026-11-12; nothing reserves them. Re-check before each release.
- **Public relays differ, and change.** The live check of 2026-11-12 (`live_exchange_nostr`, one throwaway key per relay) found `relay.primal.net` and `offchain.pub` taking and serving both kinds, replacing and deleting the offer, and taking a burst of 20 writes; `nostr.oxtr.dev` the same but rate-limiting after 11 quick writes and ignoring the deletion; `relay.damus.io` refusing the WebSocket with HTTP 503; `nos.lol` and `nostr.mom` never answering our events, on two runs. A second run found `nostr.bitcoiner.social` and `relay.nostr.net` doing everything and `relay.mostro.network` everything but the deletion. The defaults are the six that passed; the list holds more relays than the quorum needs, and the check is rerun before each release.
- **One key, two signature schemes.** The relay key signs ECDSA (inner) and Schnorr (event). libsecp256k1 derives the nonces differently (RFC 6979 and BIP-340's tagged hash); no formal proof of joint security was found. The inner signature can be dropped once `namehold-relay` support is retired.
- **The watchtower** has no Nostr equivalent: it needs chain nodes, so it stays opt-in on a `namehold-relay`.

## 6. Pointers

- Research: `.mimocode/research/2026-11-12-nostr-transport.md` (local).
- Code: `src-tauri/src/exchange_io/nostr/`, `exchange_io/env.rs`, `exchange_io/job.rs`, `commands/exchange.rs`.
- Tests: `src-tauri/src/tests/exchange_nostr_tests.rs`; live: `live_exchange_nostr` (gated by `HNS_IT_NOSTR=1`, publishes short-lived test events to the default relays).
- Docs: `docs/RELAY.md`, `docs/USER_MANUAL.md` §10c, `SECURITY.md` (exchange), CHANGELOG.
