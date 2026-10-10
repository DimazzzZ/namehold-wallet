# Security Policy

## Reporting a vulnerability

If you discover a security vulnerability, please email the maintainer directly
(see the repo's commit history for contact) with:

- A clear description of the issue
- Steps to reproduce (if applicable)
- The impact and severity you assess
- Any suggested fixes

We aim to acknowledge reports within 48 hours and provide an initial assessment
within one week. Vulnerabilities are not disclosed publicly until a fix is
available and users have had reasonable time to upgrade.

---

## Threat model

### What the wallet protects

| Asset | At-rest protection | In-memory protection | Renderer isolation |
|-------|-------------------|---------------------|-------------------|
| BIP39 seed phrase | Argon2id + AES-256-GCM under user passphrase (`noncustodial/vault.rs`) | Time-boxed session; zeroized on lock (`noncustodial/session.rs`) | Never crosses into the webview |
| Namebase session cookie | AES-256-GCM under OS-keyring-held DEK (`noncustodial/cookie_vault.rs`) | Held only during the HTTP request | Redacted from `get_settings`; write-denied from renderer |
| hsd node RPC api-key | Redacted from `get_settings`; write-denied from renderer | Held in `NodeRpcClient` struct | Never sent to the webview |
| hsd node RPC api-key (transport) | N/A (not stored encrypted) | `guard_transport` rejects remote cleartext HTTP when a key is set; HTTPS required per [hsd API guidance](https://hsd-dev.org/api-docs/#authentication) | N/A |
| Background sync daemon (`namehold-syncd`) | N/A — daemon holds no secrets and never has access to key material | Reads hsd RPC via the shared settings (api-key never leaves the Rust process); never signs or broadcasts; on mainnet, publishes listings already signed to LearnHNS Market | Rust binary — never touches the webview |

---

## The Namebase migration feature

> **Retired.** The legacy Namebase platform (sunset.namebase.io) shut down on 1 October 2026 and answers every request with 410, so the wallet no longer asks for a cookie. A cookie stored before then is still protected as described below until you log out, which clears it. The analysis is kept for that case.

### Why the cookie is required

Namebase Sunset (the custodial domain registry) offers no API-token or OAuth
mechanism. The session cookie is the only bearer credential Namebase accepts
programmatically. This is a limitation of Namebase, not a design choice by
Namehold.

### Attack surfaces and mitigations

#### 1. Compromised main webview (XSS, malicious extension)

**Mitigations (3 layers):**

- Cookie never sent to renderer: `get_settings` redacts the raw cookie and
  emits only `__has_namebase_cookie: "true"` (`commands/settings.rs:12-16`)
- Renderer cannot write the cookie or base URL: both keys are in
  `RENDERER_WRITE_DENYLIST` (`security.rs:20`); `update_setting` rejects
  writes before any DB mutation
- Per-transaction confirmation in a separate Rust-owned window: before signing
  any transaction, `sign_tx_draft` requires explicit user confirmation
  (`commands/tx.rs::sign_tx_draft_inner`, `prompt_secure`) showing action, recipient, amount, fee, txid, and
  warnings

#### 2. Local disk read (malware, laptop theft, backup snapshot)

**Mitigation:** The cookie is encrypted at rest under an OS-keyring-held DEK
(data-encryption key). The DEK is stored in the OS keyring (macOS Keychain /
Windows Credential Manager / Linux Secret Service) and cannot be extracted
without the user's OS-login credentials.

Blob format: `NBC1(4) || nonce(12) || AES-256-GCM(cookie || tag)`, hex-encoded
in the `namebase_cookie_v1` setting. The ciphertext is useless without the DEK.

**Residual risk (honest disclosure):** An attacker with concurrent code
execution as the logged-in user CAN read the DEK from the OS keyring (the OS
trusts the logged-in user). Encryption-at-rest defeats *offline* attackers, not
attackers who have already compromised the running user session. This is the
standard threat model for OS-keyring-backed secrets (Signal Desktop, Bitwarden,
VS Code, etc.).

#### 3. Network redirect (poisoned `namebase_base_url` setting)

**Mitigations:**

- Release builds: `test_base_url_override` returns empty
  (`commands/namebase.rs:24-27`); the base URL is compile-time locked to
  `https://sunset.namebase.io`
- Debug/test builds: `validate_base_url` (`namebase/client.rs:184`) enforces a
  strict host allowlist + HTTPS requirement; loopback accepted only under
  `cfg(debug_assertions)`
- Renderer cannot write the setting: `namebase_base_url` is in
  `RENDERER_WRITE_DENYLIST`

#### 4. hsd node RPC api-key sent over cleartext HTTP

The [hsd API documentation](https://hsd-dev.org/api-docs/#authentication)
explicitly states: *"If you intend to use API via network and setup api-key,
make sure to setup ssl too."*

**Mitigation:** `guard_transport` (`noncustodial/rpc.rs:139`) enforces this
requirement:

- `https://` is accepted for any host (key retained)
- `http://` is accepted only for loopback addresses (`127.0.0.1`, `::1`,
  `localhost`)
- Remote `http://` with a non-empty api-key is **rejected** with a clear error
- If `NodeRpcClient::new` is called with a remote HTTP URL, it blanks the
  api-key defensively (`rpc.rs:172-184`) so the key is never sent cleartext
  even if the guard is somehow bypassed

This fully complies with the hsd node API's security guidance.

### The lower-risk alternative (documented, not enforced)

Users who want zero credential exposure on their device can use Namebase's own
web UI to initiate transfers/withdrawals to an address generated in Namehold,
then use Namehold's chain-monitoring to confirm arrival. This workflow:

- Requires no session cookie to be stored locally
- Trades batch operations and in-app visibility for zero credential exposure
- Is fully supported by Namehold's core functionality (address generation,
  chain monitoring, name tracking)

### Feature scope (bounding the blast radius)

The Namebase session cookie is used **only** by the migration helper
(`commands/namebase.rs`). It is **never** needed for the wallet's core
non-custodial operation (holding keys, signing transactions, broadcasting via
hsd RPC, tracking owned names). A user who disconnects Namebase loses zero
wallet functionality.

---

## Background sync daemon

### What the daemon does

The background sync daemon (`namehold-syncd`) is a separate Rust binary that:

- Runs every 60 seconds when "Sync in background" is enabled (Settings →
  Connections, default ON).
- Reads wallet profiles (UTXOs, name states, transactions) from the local hsd
  node via RPC.
- Writes sync data to the shared SQLite database (`~/.namehold/portfolio.db`).
- Writes its process ID to `~/.namehold/syncd.pid` for lifecycle tracking.
- **Never signs transactions, never broadcasts, never touches key material.** It publishes listings already signed to LearnHNS Market (mainnet only).

### Attack surfaces and mitigations

#### 1. Daemon crashes or becomes unresponsive

**Mitigation:** The app detects a dead daemon on startup and respawns it (if
"Sync in background" is ON). A cross-process DB lock table (`sync_locks`) uses
heartbeats (every 10 seconds) and stale-lock takeover (after 30 seconds) to
detect and recover from crashes.

#### 2. Orphaned hsd after app exit

**Behavior (not a vulnerability):** When "Sync in background" is ON, hsd is not
killed when the app closes. The daemon keeps it alive for background syncing.
This is intentional — the next app launch adopts the running hsd. To stop hsd,
disable "Sync in background" or manually click **Stop hsd** in Settings →
Connections.

**Residual risk (honest disclosure):** An attacker with local code execution as
the app user could potentially interact with the orphaned hsd node (e.g., via
RPC) if they know the API key. This is no worse than if the user left hsd
running manually. Mitigations: run hsd on loopback only (`127.0.0.1`), use a
strong API key, and disable "Sync in background" if you're concerned about
orphaned processes.

#### 3. Daemon reads stale or corrupted sync data

**Mitigation:** The DB lock table ensures only one reader/writer is active at a
time. The app's manual Sync and the daemon coordinate via heartbeats and stale
takeover.

#### 4. The daemon never signs or broadcasts

The background daemon never signs or broadcasts; it publishes listings already signed to LearnHNS Market.

**Guarantee:** The daemon never has access to key material, never signs
transactions, and never broadcasts. Even if the daemon is compromised, it cannot
steal funds or sign malicious transactions. It can read the node (and the explorer),
write sync data and publish to LearnHNS Market what the app already signed.

The "never broadcasts" half is a runtime rule, not a property of the build: the daemon runs the same `run_sync_steps` as the app, and the Shakedex purchase refresh in it holds the one path that can send (a purchase's single rebroadcast, already signed by the app). That path is closed for the daemon by `Rebroadcast::Never`: `daemon_sync_makes_no_send_call_where_the_apps_sync_does` runs the daemon's own `sync_profile` against a mock hsd holding a purchase due for its rebroadcast and checks that no `sendrawtransaction` reaches it, while the app's sync of the same wallet sends one; `daemon_never_rebroadcasts_and_leaves_it_to_the_app` checks the refresh itself under `Rebroadcast::Never`. Signing stays impossible in the daemon: it has no key material.

Publishing (R23, R25, R28) is the one outbound write it makes besides the database, on a mainnet profile only: `shakedex_jobs.rs::{publish_listings_step, keep_listed_step}` upload a published listing's current step from the listing file Finalize & sign (or Lower price) wrote and the app signed, after re-verifying every step over the lock coin hsd reports; post the day-0 pending listing; and report a mined cancel or sale. They hold no key and name no signing call (`shakedex_layering_tests::shakedex_jobs_hold_no_signing_call`), and `shakedex_jobs_tests::daemon_publishes_but_never_signs_or_broadcasts` runs `run_sync_steps` as the daemon and as the app and checks the upload of the stored step, no `sendrawtransaction` and no draft created or changed. Each write goes through `market/learnhns.rs::LearnHnsClient`, which refuses it unless the client was given the profile's network and it is mainnet; the jobs build no client off mainnet.

---

## SPV mode

### What SPV mode does

SPV (Simplified Payment Verification) mode runs hsd with `--spv` instead of
`--index-address --index-tx`. This means:

- **No address indexing** — the node doesn't track which coins belong to which address.
- **No transaction indexing** — the node doesn't store full transaction details.
- **Read-only** — the wallet cannot send transactions in SPV mode.
- **Explorer-dependent** — balance and name data come from the configured explorer.

### Security implications

1. **Explorer trust**: In SPV mode, the wallet trusts the explorer for balance and
   name data. If the explorer is compromised, it could show incorrect data. Mitigation:
   configure a trusted explorer and optionally set a fallback URL.

2. **No local verification**: Unlike full node mode, SPV mode doesn't verify
   transactions locally against the full chain. The explorer is the source of truth
   for reads.

3. **Read-only guarantee**: SPV mode blocks all write operations (sending, name
   actions) at the write-capability check level. Even if the explorer is compromised,
   it cannot trick the wallet into signing malicious transactions.

4. **hsd still runs locally**: The SPV node still runs on your machine and handles
   block header verification. The explorer is only used for data reads, not for
   transaction validation.

### When to use SPV mode

- **Quick setup**: When you want to see your balance immediately without waiting for
  full sync.
- **Low-disk environments**: When you can't afford ~15GB for a full node.
- **Monitoring**: When you only need to watch names/auctions without sending.

### When NOT to use SPV mode

- **Sending transactions**: Switch to full node mode to send.
- **High-security requirements**: Full node mode provides stronger security guarantees.

---

## Buying names through Shakedex

### What the Market does

The Market lists names for sale through Shakedex, read from the LearnHNS Market (`https://market.learnhns.com`) or from a listing file the user imports. Buying one signs a transaction that spends the seller's lock coin with the seller's presigned signature and pays the seller from the wallet's own coins.

### Attack surfaces and mitigations

#### 1. A malicious or compromised market

**Mitigation:** Nothing the market says is trusted. Every listing is checked against the profile's own node before Buy is offered and again when the purchase is built: the lock coin must exist, hold this name and match the listing's lock address, and every price step's signature must verify. Just before broadcast the price is checked again, and the purchase is refused, and its draft discarded, if a cheaper step has become valid since it was reviewed, the step it pays is no longer valid (the median time went back, as in a reorg or on a node behind), or the node cannot say (no median time, no answer): the user reviews and signs it again. The one automatic rebroadcast of a purchase that went missing checks it the same way: at a dropped price the purchase is not sent again, and is given up with nothing paid. SPV and Explorer profiles cannot buy, because they have no node to check against.

#### 2. Network redirect or an oversized reply

**Mitigation:** The client talks only to the LearnHNS host over HTTPS and follows no redirects. Replies are capped (8 MiB for market pages, 256 KiB for a listing file). The base URL override (`learnhns_base_url`) is read only in debug builds, accepts only the LearnHNS host or loopback, and the renderer cannot write it (`RENDERER_WRITE_DENYLIST`).

#### 3. Signing an input the wallet does not own

**Mitigation:** The seller's lock coin is a foreign input carrying its own witness; the wallet signs only its own inputs. A Ledger signs every input as the wallet's own P2WPKH, so a plan with a foreign input, a custom sequence or a lock time is refused twice: when the draft is signed (`commands::tx`) and in the Ledger signer itself (`providers::ledger::signing`), which sees only the plan's shape. Draft signing refuses every Shakedex draft for a Ledger by its action as a class (`shakedex::is_shakedex_action`), because the FINALIZE into a lock is an ordinary plan the device signer cannot tell from any other; a plan with a lock-key or foreign input is refused by its shape. The wallet itself spends a lock coin of ours only by a cancel, signed with the lock key, which the signer derives from the seed at that moment and never stores; a buyer spends it with a price step the lock key signed at Finalize & sign (5). Locking a name (day 0) derives the name's lock key from the unlocked seed as well, only to read its public key and run the R18 self-check, whose test signature is over a template with a zero outpoint and is checked and dropped; only the public key is stored on the listing, and the TRANSFER commits to the program of the key just checked. The signer refuses an input that is both foreign and lock-key (`sign_plan_tx`), and refuses a lock-key input unless it is a `0x83` (`ANYONECANPAY|SINGLE`) input with a final sequence in a plan with lock time 0, from a receive-branch path at an unhardened index, whose output at the same index is a TRANSFER at that key's lock address with exactly 4 items, of this name (item 0 is the name hash), to a version-0 address (item 2 is `00`) whose hash (item 3) is our own receive address re-derived from the input's path (`noncustodial/shakedex/cancel.rs::check_lock_key_input`).

#### 4. The daemon and purchases

**Guarantee:** The background daemon refreshes each purchase's state from the chain but never rebroadcasts one. A purchase that went missing is rebroadcast at most once, and only by the app's own sync, through the same broadcast gates as every other send.

#### 5. Price steps signed by the lock key (selling)

**Mitigation:** The lock key signs a price step (`0x84`, `SINGLEREVERSE|ANYONECANPAY`) only in `commands/shakedex.rs::finalize_and_sign_confirmed`, behind the `shakedex_finalize_and_sign` command, and in `lower_price_confirmed`, behind `shakedex_lower_price`, each only after the user confirms its own prompt in the secure window. Before the prompt it makes one `0x84` signature, the R18 self-check's (`sell::lock_self_check`), over a test template with a zero outpoint: it is verified and dropped, never stored or sent. The prompt is `commands/secure_confirm.rs::confirm_rows`, on `prompt_secure`: its own `confirm` request, not the transaction prompt, listing the FINALIZE's network fee, every step with its price and when it becomes valid, the payment address, the lock address and the warning that each signature lets anyone buy at its price until a cancel is mined. A declined prompt signs and writes nothing. Before it asks, `prepare_lock_finalize` refuses unless the listing is the active profile's and ReadyToFinalize, the node reports the name still held, unrevoked, by the lock TRANSFER after its lockup, and that confirmed TRANSFER commits to the lock of the key re-derived from the seed for the name, whose public key must be the one stored on the listing (R18; the key is derived after every node read, held in memory until the command returns, never stored). Each step's template has one input, the lock coin the FINALIZE into the lock creates (that FINALIZE's txid, which Handshake fixes before signing, and the index of its one FINALIZE output paying the lock address, `sell::lock_output`), with that output's value, and one output paying the price to the listing's reserved payment address; its lock time is the node's median time minus 512 seconds, read from the node just before the prompt (with the other node reads, not after it), and nothing is signed when the node does not report a median time. `sell::sign_step` signs that template with the lock key and refuses a signature that does not verify against it (sighash `0x84`, low-S, the lock key's public key). The FINALIZE and every step are signed in one hold of the unlocked session; the signed FINALIZE draft, the steps, the listing file and the listing's move to Finalizing are written in one database transaction, and nothing is sent: `commands/tx.rs::broadcast_tx_draft` sends the FINALIZE through the usual broadcast gates. A step over a FINALIZE that is never mined can never be spent, since any other FINALIZE has another txid; `shakedex_export_listing_file` refuses to hand out the listing file while the listing is Locking, ReadyToFinalize or Finalizing, that is before its FINALIZE is mined, and for a listing that is over (Aborted, Expired, Cancelled, Sold) or Restored. Lower price asks first that the listing be Listed, a Buy Now with signed steps here and the active profile's, the node show its lock coin unspent, mined, a FINALIZE of the name at the lock address of the stored key and of the name's live registration (the same checks the cancel makes, `lock_on_node`), the new price be below the current step at the node's median time, and the re-derived lock key's public key be the stored one; its prompt shows the current and the new price, when the new one is valid, the payment address, the lock address and the warning that the earlier, dearer signatures stay valid; it signs one step over the stored lock coin paying the listing's payment address, verified before it is kept, and stores it with the rewritten listing file, read back by the strict parser, only while the listing is still Listed on that lock coin with the steps it read. Nothing is sent. Restore a lock by name (R32, `shakedex_restore_lock`) derives the lock key from the unlocked session only to read its public key and lock address, after the node reads; it signs nothing. Importing a Restored lock's own listing file (R32, `shakedex_import_own_listing_file`) derives it the same way, after the lock coin is read, only to compare its public key with the file's; it signs nothing either. The lock key signs a cancel (`0x83`) only in `commands/shakedex.rs::cancel_listing_confirmed`, behind `shakedex_cancel_listing`, after the user confirms the cancel's own prompt in the secure window (the fee, the current price, where the name comes home to, that a buyer can still take the current price until the cancel is mined, and that a purchase already in the node's mempool beats it, and for a published listing that LearnHNS Market is told the listing is cancelled at the next sync after the cancel is mined). Before it asks, `prepare_cancel` refuses unless the listing is the active profile's and Listed or Restored, the node shows its lock coin unspent, mined and a FINALIZE of the name at the lock address of the stored key, of the name's live registration, the cancel address is the profile's own receive address at the stored index under its account (`queries::receive_address_at`), the re-derived lock key's public key is the stored one, and the plan is for that account and path, its lock input marked for this name's lock key and spending the stored lock coin (`cancel::check_cancel_plan`); the signer then re-derives the destination from the input's path and refuses any other TRANSFER (`check_lock_key_input`). The signed cancel and the listing's move to Cancelling are written in one database transaction, and nothing is sent.

#### 6. Publishing to LearnHNS Market (selling)

**Scope:** Only a listing whose "Publish on LearnHNS Market" box was ticked, on a mainnet profile. `market/learnhns.rs` refuses every write (`upload_proof`, `post_pending_listing`, `refresh_status`) unless the client was given the profile's network and it is mainnet (`learnhns_tests::no_http_off_mainnet`), and the jobs make no market call at all off mainnet (`shakedex_jobs_tests::{nothing_published_off_mainnet, nothing_kept_off_mainnet}`). Publishing makes the name, the price and the lock public, and the market sees the device's IP address on every upload, post and hourly check.

**Day 0:** The pending listing (the name, the lock TRANSFER's outpoint, the lock address and the mode; no price) is posted once the lock TRANSFER's draft is `broadcast_pending`, `broadcasted` or `confirmed`, or the listing is already ReadyToFinalize or Finalizing (`db/queries.rs::list_listings_to_announce`). `broadcast_pending` is a send whose outcome the transport left unknown, so with the box ticked a pending listing can be posted for a TRANSFER the node never took. LearnHNS Market has no endpoint to withdraw a pending listing; it reads the row's status from the chain, and nothing is sent to it when the transfer is aborted.

**Uploads are unauthenticated:** `POST /api/upload-proof` takes no credential, so anyone holding a valid copy of our listing, or any other proof for the name, can upload it in place of ours. Keeping the listing on the market (R25, `shakedex_jobs.rs::keep_listed_step`) is the only defence: about hourly the market's copy (`GET /listing/<name>/proof.json`) is compared with the step we would upload (`listing_file::same_market_listing`: name, lock outpoint, public key, payment address, one step with the same price, lock time and signature, no fee address in effect, the same `expiresAt`), and ours is uploaded again unless they match. A copy naming a fee address is treated as someone else's, because `0x84` does not commit to a fee output.

**Only the market's own answer is a verdict:** a reply counts only when its body is the market's JSON shape; an HTML page, a redirect, a 5xx, a timeout or a body without the market's fields is no answer. No answer, and the market's own reply that it has not seen it on chain yet, are retried with backoff (5 minutes doubling to 6 hours); the market's own refusal is kept with its words and not retried until what would be sent changes.

**Nothing unverified is uploaded:** before an upload every stored step is re-verified over the lock coin hsd reports, at the value it reports (`shakedex_jobs.rs::market_copy`); a step that does not verify is never uploaded and the listing is marked steps unverified (`shakedex_jobs_tests::steps_not_signed_by_the_lock_are_not_uploaded`). The jobs hold no key and sign nothing (4 in "Background sync daemon").

---

## Mitigations reference table

| Concern | Mitigation | Location | Tests |
|---------|-----------|----------|-------|
| Renderer reads cookie | Redacted; presence marker only | `settings.rs:12-16` | `settings_cmd_tests` |
| Renderer writes cookie | `RENDERER_WRITE_DENYLIST` | `security.rs:20` | `settings_cmd_tests` |
| Renderer writes base URL | `RENDERER_WRITE_DENYLIST` | `security.rs:20` | `settings_cmd_tests` |
| Base URL redirect (production) | Release build ignores setting | `namebase.rs:24-27` | `namebase.rs::tests` |
| Cookie on disk (offline attacker) | AES-256-GCM under OS-keyring DEK | `cookie_vault.rs` | `cookie_vault::tests` |
| Signing without confirmation | Rust-owned secure window | `tx.rs::sign_tx_draft_inner` (`prompt_secure`) | `tx_lifecycle_tests` |
| RPC api-key sent cleartext | `guard_transport` rejects remote HTTP | `rpc.rs:139-168` | `rpc.rs::tests` |
| Audit log leaks secrets | Redacted to `***` on write; re-redacted on read | `settings.rs:40-41, 68-69` | `settings_cmd_tests` |
| Market redirect or host swap | HTTPS LearnHNS host only, no redirects, override debug-only | `market/learnhns.rs` | `learnhns_tests` |
| Tampered listing or price | Verified on the profile's node; price re-checked before broadcast | `noncustodial/shakedex/verify.rs` | `shakedex_verify_tests`, `shakedex_cmd_tests` |
| Ledger signs a foreign input, a lock coin or a Shakedex draft | Draft signing refuses every Shakedex draft by action and a lock-key or foreign plan by shape; the device signer refuses foreign, lock-key and custom-sequence or lock-time plans by shape only, and cannot see the action | `commands/tx.rs`, `providers/ledger/signing.rs` | `ledger_plan_guard_tests` |
| Lock key signs anything but a price step or a cancel | Signer accepts only a `0x83` input whose same-index output is a TRANSFER of the name at its lock address to our own version-0 receive address, and refuses an input that is both foreign and lock-key; price steps (`0x84`) are signed only by Finalize & sign, over the lock coin its own FINALIZE creates, and by Lower price, over the stored lock coin the node shows unspent and mined, each after its own secure-window confirmation, paying the listing's payment address, each verified before it is kept | `noncustodial/shakedex/cancel.rs` (`check_lock_key_input`), `noncustodial/actions.rs` (`sign_plan_tx`), `commands/shakedex.rs` (`finalize_and_sign_confirmed`, `prepare_cancel`, `cancel_listing_confirmed`, `lock_on_node`, `lower_price_confirmed`), `noncustodial/shakedex/sell.rs` (`sign_step`, `lock_output`) | `actions::tests::sign_plan_refuses_a_lock_key_input_that_is_not_a_cancel`, `shakedex_sell_tests::{lock_then_finalize_and_sign, refuses_finalize_on_commitment_mismatch, confirmation_lists_every_step, cancelled_confirmation_writes_nothing, cancel_commits_only_to_the_listings_reserved_receive_address, cancel_needs_the_unlocked_signer, lower_price_becomes_current, lower_price_needs_the_unlocked_signer}`, `cancel::tests::cancel_plan_is_for_the_profiles_account_and_path` |
| Name locked to a key that cannot move it | The lock key is re-derived and self-checked (script, program, address, a test `0x84` signature) before the TRANSFER draft is written; the TRANSFER commits to that key's program | `noncustodial/shakedex/sell.rs` (`lock_self_check`, `lock_transfer_covenant`), `commands/shakedex.rs` (`build_lock_draft_inner`) | `shakedex_sell_tests::{refuses_lock_on_self_check_failure, lock_draft_commits_to_the_derived_lock_address}`, `shakedex_vector_tests::lock_transfer_covenant_matches_hsd` |
| Name finalized into a lock this wallet cannot move | Finalize & sign re-derives the lock key after the node reads and refuses unless the node's confirmed lock TRANSFER commits to that key's lock and the stored public key is that key's (R18) | `commands/shakedex.rs` (`prepare_lock_finalize`) | `shakedex_sell_tests::refuses_finalize_on_commitment_mismatch` |
| Listing file out before its FINALIZE is mined | Export refuses in Locking, ReadyToFinalize and Finalizing | `commands/shakedex.rs` (`export_listing_file_from_conn`) | `shakedex_sell_tests::listing_file_is_exported_only_once_the_finalize_is_mined` |
| Daemon rebroadcasts a purchase | `SyncCaller::Daemon` never rebroadcasts | `commands/sync.rs`, `shakedex_jobs.rs` | `shakedex_purchase_state_tests::{daemon_sync_makes_no_send_call_where_the_apps_sync_does, daemon_never_rebroadcasts_and_leaves_it_to_the_app}` |

---

## Dependency auditing

CI runs an advisory-only audit job (`cargo audit` + `pnpm audit --prod`) on
every PR to surface newly-disclosed CVEs in the dependency graph.

### Suppressed advisories

A suppressed advisory is one we have reviewed and determined does not apply to
this app (or is unfixable because it is frozen inside an upstream dependency's
transitive graph). Every suppression MUST be justified here.

#### Frontend (`pnpm audit`)

Suppressions live in `pnpm-workspace.yaml` under `auditConfig.ignoreGhsas`.

| Advisory | Package | Rationale |
|----------|---------|-----------|
| [GHSA-qwww-vcr4-c8h2](https://github.com/advisories/GHSA-qwww-vcr4-c8h2) | `react-router` | CSRF bypass that the advisory states "only affects your application if you are using the unstable RSC APIs". Namehold is a Tauri single-page app with client-side routing only — it does not use React Server Components, so the vulnerable code path is never reached. Revisit when upgrading to `react-router@>=8.3.0`. |

#### Backend (`cargo audit`)

Suppressions live in `src-tauri/.cargo/audit.toml` under `[advisories] ignore`.
Every crate below is a transitive dependency of Tauri v2 — none are direct
dependencies of this crate.

| Advisory | Package | Kind | Rationale |
|----------|---------|------|-----------|
| [RUSTSEC-2026-0194](https://rustsec.org/advisories/RUSTSEC-2026-0194), [RUSTSEC-2026-0195](https://rustsec.org/advisories/RUSTSEC-2026-0195) | `quick-xml 0.39.4` | vuln (DoS) | Fixed only in `>=0.41.0` (semver-major). Pinned to `0.39.x` by `plist` (`^0.39.2`) and `wayland-scanner` (`^0.39`) inside Tauri v2. Not exposed to untrusted XML at runtime: `plist` parses the app's own macOS `Info.plist` at bundle time; `wayland-scanner` parses local Wayland protocol XML at build time. Revisit when Tauri's tree admits quick-xml `>=0.41`. |
| [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429) | `glib 0.18.5` | unsound | Unsound `VariantStrIter` iterators. Pulled in by Tauri v2's wry/tao GTK3 Linux backend; not reachable from app code. |
| [RUSTSEC-2024-0370](https://rustsec.org/advisories/RUSTSEC-2024-0370) | `proc-macro-error 1.0.4` | unmaintained | Compile-time-only proc-macro helper via `glib-macros` / `gtk3-macros`. No runtime code. |
| [RUSTSEC-2024-0411](https://rustsec.org/advisories/RUSTSEC-2024-0411) through [-0420](https://rustsec.org/advisories/RUSTSEC-2024-0420) | gtk-rs GTK3 bindings (`atk`, `atk-sys`, `gdk`, `gdk-sys`, `gdkwayland-sys`, `gdkx11`, `gdkx11-sys`, `gtk`, `gtk-sys`, `gtk3-macros`, all `0.18.2`) | unmaintained | Tauri v2 Linux windowing depends on the GTK3 stack. A fix requires Tauri migrating to GTK4/webkitgtk-6 upstream. |
| [RUSTSEC-2025-0075](https://rustsec.org/advisories/RUSTSEC-2025-0075), [-0080](https://rustsec.org/advisories/RUSTSEC-2025-0080), [-0081](https://rustsec.org/advisories/RUSTSEC-2025-0081), [-0098](https://rustsec.org/advisories/RUSTSEC-2025-0098), [-0100](https://rustsec.org/advisories/RUSTSEC-2025-0100) | `unic-*` (`unic-char-range`, `unic-common`, `unic-char-property`, `unic-ucd-version`, `unic-ucd-ident`, all `0.9.0`) | unmaintained | Pulled in transitively via `urlpattern` <- `tauri-utils`. No direct use. |

> `anyhow` (RUSTSEC-2026-0190) was fixed by bumping to `1.0.104` rather than
> suppressed, since it had a semver-compatible patch.

---

## Disclosure policy

- Vulnerabilities are not disclosed publicly until a fix is available
- Security updates are released as soon as feasible
- Researchers who report responsibly are credited (unless they prefer anonymity)

---

## Additional resources

- [README](./README.md) -- feature overview and architecture
- [User Manual](./docs/USER_MANUAL.md) -- user-facing security guidance
- [CHANGELOG](./CHANGELOG.md) -- security fixes and improvements
- [hsd API docs](https://hsd-dev.org/api-docs/) -- Handshake node RPC reference
| Daemon crashes mid-sync | Heartbeat every 10s; stale-lock takeover after 30s; app respawns daemon on next startup | `db/sync_lock.rs`, `commands/daemon_ctl.rs` | `sync_lock` tests |
| Concurrent writes by app + daemon | Cross-process `sync_locks` table; app acquires with priority, daemon preempts stale locks | `db/sync_lock.rs`, `commands/sync.rs` | `sync_lock`, `sync_race` tests |
| Daemon signs (would-be) | Daemon has no access to key material — it reads hsd, writes sync data and, on mainnet, publishes listings the app already signed; the market jobs name no signing call | `bin/namehold-syncd.rs`, `daemon/mod.rs`, `shakedex_jobs.rs` | (no key material in the daemon process), `shakedex_layering_tests::shakedex_jobs_hold_no_signing_call` |
| Daemon broadcasts (would-be) | The one send path in `run_sync_steps` (a purchase's rebroadcast) is closed for the daemon at runtime by `Rebroadcast::Never` | `commands/sync.rs`, `shakedex_jobs.rs` | `shakedex_purchase_state_tests::{daemon_sync_makes_no_send_call_where_the_apps_sync_does, daemon_never_rebroadcasts_and_leaves_it_to_the_app}`, `shakedex_jobs_tests::daemon_publishes_but_never_signs_or_broadcasts` |
| Daemon publishes to the market | Mainnet only; already-signed steps re-verified on hsd; no key in the jobs | `market/learnhns.rs`, `shakedex_jobs.rs` | `learnhns_tests::no_http_off_mainnet`, `shakedex_jobs_tests::{daemon_publishes_but_never_signs_or_broadcasts, steps_not_signed_by_the_lock_are_not_uploaded}` |
| hsd left running after app exit | Intentional when "Sync in background" ON; hsd bound to loopback + api-key required | `lib.rs` (setup/exit hooks) | `settings-background-sync` tests |
