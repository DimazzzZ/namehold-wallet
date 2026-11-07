# HNS ⇄ BTC exchange

**Status:** draft (2026-11-07), being implemented on branch `feat/exchange`. Not released. Requirements marked _(planned)_ have no code yet; every other requirement names the code that enforces it and the test that pins it.

## 1. Summary

People who want to buy or sell HNS have nowhere to meet. This feature adds one place inside Namehold: an order book of HNS ⇄ BTC offers, and a trade that settles itself as a cross-chain **atomic swap** — either both sides get paid, or both get their own coins back. Nobody holds anyone's coins: not the relay that carries the offers, not the watchtower, not the project. Each party confirms **once**, in the secure window, and that one confirmation signs every transaction their side of the trade can ever need: the payment into the lock, the refund out of it, and the claim of the other side's coins. After that, nothing needs the recovery phrase again. The background daemon and an optional watchtower only send transactions the user already signed, and neither can redirect a coin. Everything a party relies on about the other chain is checked by the party itself: the HNS chain by the profile's own node, Bitcoin by proof of work over block headers and a Merkle proof, from two independent sources.

## 2. Terms

**Seller** — the party that pays HNS and receives BTC. **Buyer** — the party that pays BTC and receives HNS. Either can be the maker.

**Offer** — a maker's signed announcement on the relay: a side (`sell` = maker is the Seller, `buy` = maker is the Buyer), a price, the smallest and largest trade, and the maker's offer key. An offer is live only while the maker's Namehold renews it (heartbeat); a silent offer expires.

**Maker / Taker** — who published the offer / who takes it.

**Trade** — one swap between a maker and a taker, identified by its **terms id**.

**Terms** — everything both scripts are made from, agreed before any coin moves: networks, `hns_amount` (dollarydoos), `btc_amount` (satoshis), the secret's hash `H`, the four public keys (Seller's HNS refund key, Buyer's HNS claim key, Buyer's BTC refund key, Seller's BTC claim key), the trade's start `t0`, the Seller's funding deadline, the two refund times, the confirmation counts. The terms id is SHA-256 of their canonical encoding.

**Secret** (`x`) — 32 bytes chosen by the Buyer, derived from the Buyer's recovery phrase and the trade, so a restore can recompute it. `H = SHA-256(x)`.

**BTC lock** — the Buyer's BTC, paid into a P2WSH output the Seller can take with the secret and the Seller's key, or the Buyer can take back after the BTC refund time.

**HNS lock** — the Seller's HNS, paid into a P2WSH output the Buyer can take with the secret and the Buyer's key, or the Seller can take back after the HNS refund time.

**Commit** — a party's one confirmation in the secure window, which signs all of that party's transactions for the trade. The Buyer commits first, the Seller second.

**Reveal** — the Buyer's claim of the HNS lock. Its witness carries `x`, so it publishes the secret.

**Claim** — taking the other side's lock with the secret. **Refund** — taking your own lock back after its refund time.

**Presigned transaction** — a transaction signed at Commit and kept for later. A presigned claim has an empty slot in its witness for `x`; the signature does not cover the witness, so whoever learns `x` can fill the slot, but nobody can change where the coins go.

**Fee ladder** — the same Bitcoin claim or refund presigned at several fee rates (1×, 2×, 4× … of the rate at Commit), each replacing the cheaper one (BIP 125) if it is not mined in time.

**Watchtower** — a service that holds a party's presigned transactions and sends them at the right moment: the Seller's claim of the BTC lock as soon as the secret appears on the HNS chain, and each party's refund once its time has come. It holds no key and no secret before it is public; every transaction it can send pays the party who signed it. The daemon is the user's own watchtower; a remote one is a second, independent one.

**Relay** — the server that carries offers and trade messages. It holds no coin and signs nothing that moves one. Every message is verified by the receiving Namehold; every chain fact a message claims is re-checked on chain.

**BTC account** — a minimal Bitcoin account inside a profile: BIP 84 keys from the profile's recovery phrase (`m/84'/0'/0'` on mainnet, `m/84'/1'/0'` elsewhere), so any BIP 84 wallet restores it from the same phrase. It funds the Buyer's BTC lock, and receives the Seller's claim and the Buyer's refund.

**Swap keys** — the per-trade keys of both chains, at `m/44'/<HNS coin type>'/<account>'/3'/<trade index>/<role>`: role 0 is the HNS key (the Seller's refund key or the Buyer's claim key), role 1 the BTC key (the Seller's claim key or the Buyer's refund key). Branch `3'` follows receive (0), change (1) and Shakedex lock keys (2'). The branch's extended public key is stored, so the daemon can hand out a trade's public keys without the phrase; private keys are derived only at Commit.

## 3. The protocol

### 3.1 Scripts

Both locks use the same script shape, paid to as P2WSH (on HNS the program is SHA3-256 of the script, on Bitcoin SHA-256):

```
OP_IF
  OP_SIZE 32 OP_EQUALVERIFY OP_SHA256 <H> OP_EQUALVERIFY <claim key>
OP_ELSE
  <refund time> OP_CHECKLOCKTIMEVERIFY OP_DROP <refund key>
OP_ENDIF
OP_CHECKSIG
```

- Claim witness: `[signature, x, 0x01, script]`. Refund witness: `[signature, <empty>, script]`, with the transaction's lock time set to the refund time and its input's sequence below final.
- `OP_SIZE 32` makes a secret of any other length unusable on both chains, so a secret valid on one chain is valid on the other.
- On HNS the refund time is a time lock in hsd's encoding: bit 31 set, the low 31 bits the Unix time divided by 512, rounded up (`consensus.LOCKTIME_FLAG`, `LOCKTIME_GRANULARITY`); it is final once 512 × value is below the chain's median time past. On Bitcoin it is the Unix time itself (at least 500 000 000), final once below the median time past (BIP 113).

### 3.2 The one-Commit-per-party construction

Bitcoin signatures always commit to the outpoint they spend, so the Seller can sign the claim of the BTC lock only once the BTC lock exists. HNS accepts `SIGHASH_ALL | SIGHASH_NOINPUT | SIGHASH_ANYONECANPAY` (`0xc1`, hsd `common.isSignatureEncoding`): under it the signature commits to the witness script, the spent value, the outputs and the lock time, but not to the outpoint (hsd `tx.signatureHash`). So the Buyer can sign the claim of the HNS lock **before the HNS lock exists**: the script and its value are fixed by the terms. That gives each party exactly one Commit:

| Commit | Signs |
|---|---|
| Buyer (first) | the BTC lock's funding (from the BTC account); the BTC refund (fee ladder); the HNS claim, `0xc1`, to the Buyer's own HNS address, `x` slot empty |
| Seller (second, after the BTC lock has its confirmations) | the HNS lock's funding (from the wallet); the HNS refund; the BTC claim (fee ladder), to the Seller's BTC account, `x` slot empty |

### 3.3 Sequence

1. **Offer.** The maker publishes it (offer key, heartbeat).
2. **Take.** The taker sends a take request: amount, and its own half of the terms (the Buyer's `H` and keys, or the Seller's keys). The maker's daemon answers automatically with the other half and `t0`: public keys come from the stored swap-key extended public key; a Buyer-maker's `H` comes from a pool of hashes computed when the offer was made. No secret is used. Both sides check the terms against their own policy (§3.5) and the offer.
3. **Buyer Commit.** In the secure window. Then the BTC lock is funded and announced (`btc_locked`). The Buyer's daemon and watchtower keep the BTC refund ladder.
4. **BTC lock confirmed.** The Seller's Namehold checks the BTC lock (§3.6): exact script, exact value, the agreed refund time, the confirmation count, proof of work.
5. **Seller Commit.** In the secure window, before the Seller's funding deadline. Then the HNS lock is funded and announced (`hns_locked`). The Seller's daemon and watchtower keep the HNS refund and the BTC claim ladder.
6. **Reveal.** The Buyer's daemon sees the HNS lock on the profile's own node, with the exact script and value its presigned claim was signed for and the required confirmations, before the reveal deadline. It fills in `x` and sends the claim.
7. **Claim.** The Seller's daemon or watchtower sees the HNS lock spent, takes `x` out of the witness, checks `SHA-256(x) = H`, fills it into the BTC claim and sends it, climbing the fee ladder until it is mined.
8. **Refunds.** A lock that is still unspent after its refund time is refunded by its owner's daemon or watchtower.

### 3.4 Why nobody can lose their coins

| If … | then … |
|---|---|
| the Seller never commits | the Buyer's BTC comes back after the BTC refund time; the Seller never locked anything |
| the Seller locks the wrong amount or script | the Buyer's presigned claim does not fit it, so the Buyer never reveals; both refund |
| the Buyer never reveals | the Seller's HNS comes back after the HNS refund time, the Buyer's BTC after the BTC refund time |
| the Buyer reveals | the HNS is the Buyer's; the secret is public on the HNS chain, and the Seller has at least the claim window (12 h on mainnet) until the BTC refund time to claim |
| the Seller's Namehold is off after the reveal | the remote watchtower claims for it; the claim pays only the Seller |
| the relay lies | it can hide offers or messages; every chain fact is re-checked, so it can only waste time |
| a Bitcoin source lies | the BTC lock is accepted only with a Merkle proof into headers with valid proof of work above a floor, linked to the confirmation count, and two sources must agree (§3.6) |
| the watchtower is dishonest | it can only fail to send; everything it holds pays its owner, and the daemon sends the same transactions |

What remains is a liveness risk, not a theft: a Seller whose daemon **and** watchtowers are all offline from the reveal until the BTC refund time can lose the BTC claim. The window is 12 hours on mainnet and the app notifies at the reveal.

### 3.5 Policy (mainnet; regtest and testnet use minutes, `noncustodial/exchange/policy.rs`)

- The Buyer must fund within 1 h of `t0`; the Seller's funding deadline is `t0 + 4 h`.
- HNS refund time `t0 + 12 h`; reveal deadline = HNS refund time − 3 h; BTC refund time = HNS refund time + 12 h.
- Confirmations: BTC lock 2 (3 from 0.02 BTC), HNS lock 6 (12 from 0.02 BTC worth).
- Limits during the beta: at least 20 000 sats and at most 0.1 BTC per trade.
- HNS presigned transactions pay 20× the node's rate (at least 20 doos/byte): hsd has no replace-by-fee, so they are signed once at a rate that will always be mined.
- Bitcoin presigned ladders: 1×, 2×, 4×, 8×, 16×, 32× the source's fastest estimate (at least 2 sat/vB), dropping every rung whose fee exceeds 20 % of the coin; a trade whose first rung cannot fit is refused. The next rung is sent after 3 blocks without the previous one mined, and the highest rung once fewer than 6 h are left before the competing path opens.

### 3.6 Checking the other chain

- **HNS** is read through the profile's node, which must be a local node (full or SPV) or a remote node: the Explorer chain source cannot check a lock, and the exchange is refused on it.
- **Bitcoin**: the BTC lock is accepted only when (a) its transaction's txid is computed locally from the raw bytes, (b) its output has exactly the lock's script and value, (c) a Merkle proof places it in a block whose header has valid proof of work, (d) the required number of further headers link to that one, each with valid proof of work, (e) on mainnet every header's target is at or below a difficulty floor in the code, and (f) every configured source (two by default) reports the same block for it.

## 4. Requirements

| # | Requirement | Code | Test |
|---|---|---|---|
| R1 | Both lock scripts are exactly §3.1; a script that differs in one byte is not recognised as a lock of the terms | `exchange/hns_htlc.rs`, `exchange/btc_htlc.rs` | `exchange_hns_htlc_tests`, `exchange_btc_htlc_tests` |
| R2 | HNS refund times are encoded as hsd time locks, rounded up; the boundary is pinned on both sides | `hns_htlc::encode_time_lock` | `exchange_hns_htlc_tests::time_lock_*` |
| R3 | The Buyer's HNS claim is signed `0xc1` and stays valid for any outpoint holding the lock script and value, and for no other script or value | `hns_htlc::sign_claim` | `exchange_hns_htlc_tests`, live `live_exchange_hns_*` |
| R4 | The HNS sighash with NOINPUT and ANYONECANPAY matches hsd byte for byte | `noncustodial/tx.rs` | `exchange_hsd_vectors_tests` (vectors produced by hsd 8.0.0) |
| R5 | Bitcoin claims and refunds pass Bitcoin Core's script verifier (libbitcoinconsensus) | `exchange/btc_htlc.rs` | `exchange_btc_htlc_tests::*_passes_consensus` |
| R6 | A secret of any length but 32 bytes cannot claim on either chain | scripts | `*_refuses_a_secret_of_*` |
| R7 | The terms are checked before anything is signed: amounts and price against the offer, the beta limits, every time against the policy, keys valid and distinct, `H` 32 bytes | `exchange/terms.rs::check` | `exchange_terms_tests` |
| R8 | The Buyer reveals only if the HNS lock found on the node has exactly the script and value of the presigned claim, the required confirmations, and the reveal deadline has not passed; never otherwise | `exchange/engine.rs` | `exchange_engine_tests::buyer_*` |
| R9 | The Seller commits only after the BTC lock passes §3.6 and before the funding deadline; the command refuses otherwise | `engine::seller_may_commit`, `commands/exchange.rs` | `exchange_engine_tests::seller_*`, `exchange_btc_spv_tests` |
| R10 | A secret taken from the HNS chain is used only if its SHA-256 is `H` | `engine` | `exchange_engine_tests::seller_ignores_a_wrong_secret` |
| R11 | Refunds are sent only once the chain's median time past is past the refund time | `engine` | `exchange_engine_tests::*refund*` |
| R12 | The fee ladder escalates as §3.5 and never above 20 % of the coin | `exchange/fees.rs` | `exchange_fees_tests` |
| R13 | Everything the daemon and the watchtower send is a transaction the user signed at Commit, with at most the public or own secret put into its empty slot; neither holds a private key | `exchange/watch.rs`, `daemon` | `exchange_watch_tests`, `exchange_layering_tests` |
| R14 | Swap keys follow §2 and the stored branch public key yields the same public keys as the phrase | `exchange/keys.rs` | `exchange_keys_tests` |
| R15 | The BTC account derives the same addresses as any BIP 84 wallet from the same phrase | `exchange/btc_account.rs` | `exchange_btc_account_tests::bip84_test_vectors` |
| R16 | Each Commit shows, in the secure window, what leaves, what arrives and where, the fee, the refund times and the deadline, computed from the transactions that are signed | `exchange/review.rs` | `exchange_review_tests` |
| R17 | The exchange is refused on a watch-only or Ledger profile, on the Explorer chain source and on a node that cannot send, with the same sentence on every button | `commands/exchange.rs`, `src/lib/exchangeText.ts` | `exchange_cmd_tests`, `Exchange.test.tsx` |
| R18 | Relay messages are signed by the sender's offer or trade key and checked on receipt; a message that fails is dropped | `exchange/relay_msg.rs` | `exchange_relay_msg_tests` |

## 5. Explicitly not enforced

- The relay does not authenticate who is behind an offer; a key is only a key. Atomicity, not identity, protects the coins.
- No fiat, no stablecoins, no Lightning in this version.
- The order book is not matched by price; the taker picks an offer.
- The price is not checked against any market price.

## 6. Known gaps

- **Presence of the maker.** Every trade needs a Commit from each side; a maker whose Namehold is not open when someone takes the offer delays the trade, and if the maker is the Seller the Buyer's BTC waits for its refund time. A maker cannot pre-authorise trades: that would need the phrase in the daemon, which this design refuses.
- **The Buyer's secret on disk.** From the Buyer's Commit until the reveal the secret is stored in the database (owner-only file), because the daemon must reveal without the phrase. Someone who reads that file **and** is the Seller could take the BTC without locking HNS. The secret is never sent to a watchtower before it is public.
- **No Bitcoin regtest in CI.** Bitcoin scripts are checked by Bitcoin Core's own verifier and the chain logic against a simulated chain; a live Bitcoin regtest run is in the QA cases.
- **A lost database.** Presigned transactions live in the database and at the watchtower. A restore from the phrase alone recomputes keys and secrets but not the counterparty's terms; until then the watchtower keeps the refunds.

## 7. Pointers

- Code: `src-tauri/src/noncustodial/exchange/`, `src-tauri/src/commands/exchange.rs`, `src/pages/Exchange*`.
- Tests: `src-tauri/src/tests/exchange_*_tests.rs`, `src-tauri/src/tests/live_exchange_it.rs`.
- QA: [docs/qa/scenarios/exchange/](../qa/scenarios/exchange/).

## 8. Relay and watchtower API (v1)

JSON over HTTPS (`http://` only to this machine). Every body is at most 16 KiB; a larger one is refused with 413. Errors are `{"error": "<sentence>"}` with a 4xx/5xx status. The server keeps nothing it cannot verify: it checks every signature with `noncustodial::exchange::relay_msg` before storing. It never sees a private key or a secret, and nothing it stores can move a coin.

| Method and path | Body | Answer |
|---|---|---|
| `GET /v1/health` | — | `{"ok": true, "version": "<server version>", "networks": ["main", …]}` |
| `POST /v1/offers` | a `SignedOffer` | `201 {"id": "<offer id>"}`. Refused: bad signature, `check_shape` fails, network not served, more than 20 live offers for one maker key. Posting the same offer again is its **heartbeat**: it updates `lastSeen`. |
| `GET /v1/offers?network=<n>` | — | `{"offers": [SignedOffer…], "now": <unix>}`: offers of that network seen within the last 180 s and not expired, newest first, at most 500 |
| `POST /v1/offers/<id>/withdraw` | `{"signature": "<hex>"}`: the maker key's compact signature over SHA-256 of `"namehold exchange withdraw v1\0" ‖ <id as ASCII hex>` | `200 {}`; the offer is gone |
| `POST /v1/messages` | an `Envelope` | `201 {"id": "<sha256 of the body>"}`. Refused: bad signature, unknown version, oversized. At most 200 messages per sender key per hour. Kept 7 days. |
| `GET /v1/messages?to=<pubkey>&after=<cursor>` | — | `{"messages": [Envelope…], "cursor": "<opaque>"}` in arrival order, at most 200; `after` omitted means from the start |
| `POST /v1/watch` | a `WatchJob` | `201 {"id": "<terms id>-<role>"}`. Refused when `WatchJob::check` fails or the network is not served. Posting the same job again is a no-op. |
| `GET /v1/watch/<id>` | — | `{"status": Status, "sent": SentLog, "updatedAt": <unix>}` |

The watchtower runs `engine::step` on every job each minute, with its own HNS node and Bitcoin sources, and sends what it returns.

## 9. Commands (the Tauri bridge)

All act on the active profile. Structs are camelCase (`src/types/exchange.ts` mirrors them field for field). Amounts of HNS cross as strings of whole HNS with up to 6 decimals (`hnsAmount.ts`), BTC as integer satoshis, prices as strings of sats per HNS with up to 3 decimals, times as Unix seconds.

| Command | Arguments | Returns | Secure window |
|---|---|---|---|
| `exchange_overview` | — | `ExchangeOverview` | — |
| `exchange_enable` | — | `ExchangeOverview` | no (stores the BTC account's and the swap branch's public keys; needs the unlocked session) |
| `exchange_btc_balance` | — | `BtcBalance` | — |
| `exchange_btc_withdraw` | `to_address`, `amount_sats: number \| null` (null = everything) | `{ txid }` | yes |
| `exchange_list_offers` | — | `MarketOffers` | — |
| `exchange_create_offer` | `side: "sell" \| "buy"`, `price: string`, `min_hns: string`, `max_hns: string` | `OfferRow` | yes |
| `exchange_withdraw_offer` | `offer_id` | `null` | — |
| `exchange_my_offers` | — | `OfferRow[]` | — |
| `exchange_take_offer` | `offer_id`, `hns_amount: string` | `TradeRow` | — (nothing is signed; the Commit asks) |
| `exchange_list_trades` | — | `TradeRow[]` | — |
| `exchange_commit` | `trade_id` | `TradeRow` | yes: the Commit (§3.2) |
| `exchange_cancel_trade` | `trade_id` | `TradeRow` | — (only before this party's Commit) |
| `exchange_refresh` | — | `TradeRow[]` | — (runs one engine step for every trade now) |
| `exchange_update_settings` | `relay_url: string`, `watchtower_url: string`, `btc_sources: string[]`, `bitcoind_auth: string \| null` (null = keep) | `ExchangeOverview` | yes: one old → new row per change; the auth is shown as set/changed/cleared, never its value |

```ts
type ExchangeOverview = {
  network: string;                 // "main" | "testnet" | "regtest" | "simnet"
  available: boolean;              // false: `refusal` says why (R17)
  refusal: string | null;
  enabled: boolean;                // the BTC account and swap keys exist
  relayUrl: string;
  watchtowerUrl: string | null;
  btcSources: string[];
  btcNetwork: string;              // "bitcoin" | "testnet" | "regtest"
  bitcoindAuthSet: boolean;        // a bitcoind RPC user:password is stored
};
type BtcBalance = {
  confirmedSats: number;
  unconfirmedSats: number;
  receiveAddress: string;
  lockedInTradesSats: number;      // BTC in this profile's BTC locks
  checkedAt: number;
  error: string | null;            // the source could not be read
};
type OfferRow = {
  id: string;
  side: "sell" | "buy";            // the maker's side
  price: string;                   // sats per HNS
  minHns: string; maxHns: string;
  minSats: number; maxSats: number;
  own: boolean;
  makerKey: string;                // first 8 hex of the maker's relay key
  expiresAt: number;
  live: boolean;                   // own offers: published and heartbeating
};
type MarketOffers = { offers: OfferRow[]; fetchedAt: number; error: string | null };
type TradeRow = {
  tradeId: string;
  termsId: string | null;          // null while negotiating
  role: "buyer" | "seller";
  maker: boolean;
  offerId: string;
  hnsAmount: string;
  btcSats: number;
  price: string;
  status: TradeStatus;             // engine::Status, tagged by `kind`
  negotiating: boolean;            // terms not agreed yet
  declined: string | null;         // the counterparty's reason
  timeline: { buyerFundDeadline: number; sellerFundDeadline: number; revealDeadline: number;
              hnsRefundTime: number; btcRefundTime: number } | null;
  hnsLock: { txid: string; vout: number } | null;
  btcLock: { txid: string; vout: number } | null;
  watchtower: "none" | "registered" | "failed";
  lastError: string | null;
  createdAt: number; updatedAt: number;
};
type TradeStatus =
  | { kind: "awaitingBuyerCommit"; deadline: number }
  | { kind: "awaitingBtcLock"; confirmations: number; needed: number }
  | { kind: "awaitingSellerCommit"; deadline: number }
  | { kind: "awaitingHnsLock"; confirmations: number; needed: number }
  | { kind: "revealing" }
  | { kind: "awaitingReveal"; refundOpensAt: number }
  | { kind: "claiming"; rung: number }
  | { kind: "completed" }
  | { kind: "awaitingRefund"; chain: "hns" | "btc"; opensAt: number }
  | { kind: "refunding"; chain: "hns" | "btc" }
  | { kind: "refunded"; chain: "hns" | "btc" }
  | { kind: "expired" }
  | { kind: "cannotCheck"; chain: "hns" | "btc" }
  | { kind: "needsAttention"; reason: "sellerLockedWrongAmount" | "buyerLockedWrongAmount"
        | "btcRefundedAfterReveal" | "hnsRefundedAfterReveal" | "missingPresigned"; chain: "hns" | "btc" };
```

A trade needs this party's Commit when its status is `awaitingBuyerCommit` and the role is `buyer`, or `awaitingSellerCommit` and the role is `seller` (`src/lib/exchangeStatus.ts::needsMyCommit`).
