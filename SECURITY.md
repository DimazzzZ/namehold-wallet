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
| BIP39 seed phrase | Argon2id + AES-256-GCM under user passphrase (`noncustodial/vault.rs`); a new mainnet wallet needs a passphrase of at least 8 characters (`secure_wallet::resolve_secret_key`). A test-network wallet may have none: its phrase is then encrypted under a key in the public source, which protects nothing, as does a mainnet wallet created that way before the rule | The signer session lasts 60 to 3600 seconds after its last use (default 900) and never more than one hour after unlock, however often it is used; a stored out-of-range value is clamped when read. An expired session's key is dropped at the first check, and the app checks once a second. Locking, expiry and profile switches overwrite the master secret key and chain code in place (`noncustodial/session.rs`). Prompt answers, passphrases, recovery phrases, seeds, the vault's derived key, Argon2's working memory and the expanded AES key are overwritten when dropped, on every path including errors. Not covered: copies in serde_json buffers, the Tauri IPC and the webview, a `SecretKey` copied out of `ExtendedPrivKey` (`derive_address`, Shakedex lock keys), the AES-GCM GHASH subkey, and temporaries and moved values the compiler leaves on the stack | Never crosses into the webview |
| Namebase session cookie | AES-256-GCM under OS-keyring-held DEK (`noncustodial/cookie_vault.rs`) | Held only during the HTTP request | Redacted from `get_settings`; `update_setting` refuses it (not on `RENDERER_SETTING_KEYS`) |
| hsd node RPC api-key | Not encrypted; the data directory is owner-only | Held in `NodeRpcClient` struct | Redacted from `get_settings`; `update_setting` refuses it; written only by `update_protected_settings` after the user confirms in the secure window (the row says set, changed or cleared, never the value); redacted in the audit log. Never sent to the webview |
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
- Renderer cannot write the cookie or base URL: `update_setting` is an allowlist (`security.rs::RENDERER_SETTING_KEYS`) and refuses any other key before any DB mutation
- Per-transaction confirmation in a separate Rust-owned window, and signing bound to what it shows: before signing any transaction, `sign_tx_draft` asks for explicit confirmation in the secure window (`commands/tx.rs::sign_tx_draft_confirmed`, `prompt_secure`). A send shows its recipient, amount, fee, the total leaving the wallet and its txid, and a Max send says it sweeps the wallet. A send to an address this wallet has not paid before, and that is not its own, says so. A name action's rows are computed from the plan that is signed (`noncustodial/review.rs`): its names come from the covenants' name hashes, a transfer's recipient from the address its covenant commits to, a finalize's from the output that receives the name, a bid's value from the wallet's own record checked against the blind the BID commits to, with the amount locked until reveal, and the fee and txid from the plan's own inputs and outputs; a draft whose rows cannot be shown truthfully is refused. A revoke asks for the name to be typed, and the backend checks what was typed. Signing then refuses unless the transaction it signs is the one reviewed (`ensure_reviewed`, `ensure_signed_is_reviewed`): a send signs only the coins it reserved when it was built, never the wallet's other coins, its change must go to the account's own change address re-derived from the xpub (any plan's change likewise, for every signer), and the signed transaction's txid must be the one the window showed. Residual risk, not mitigated: a process that can write the database could rewrite a draft's plan and summary together; the window then shows that draft truthfully, but the user must read it.
- A file the backend reads at a path the renderer names (a watchlist, name-list or Namebase-history import) goes through `commands/user_file.rs::read_chosen_file`: a symbolic link or anything but a regular file is refused (`O_NOFOLLOW`), a file over 8 MiB or not UTF-8 text is refused, and the importers store only rows whose name is a Handshake name, never repeating a refused row's text, so a compromised webview cannot use an import to copy another file (`~/.hsd/hsd.conf`, a key file) into rows it reads back (`watchlist_cmd_tests::import_refuses_a_file_whose_rows_are_not_names_and_stores_nothing`, `csv_cmd_tests::import_csv_stores_only_handshake_names_and_never_echoes_a_refused_row`).
- A file the backend writes at a path the renderer names (the bid backup, `save_bid_backup`; a listing file, `shakedex_save_listing_file`; the watchlist and name-list CSV exports; the path comes from the native save dialog) goes through `commands/user_file.rs::write_chosen_file`: a symbolic link there is refused and its target left untouched (checked with `symlink_metadata`, and the open does not follow the last component, `O_NOFOLLOW`), as is anything that is not a regular file, and nothing is truncated before those checks. The bid backup is 0600 before its secrets are written. Residual risk, not mitigated: a compromised webview can still name any regular file the user's account can write (or a hard link to one) and have it replaced by a backup or a listing file; a save dialog's answer cannot be told apart from a path the page made up.

- Trust-root settings need a confirmation. `PROTECTED_SETTING_KEYS` (node URL and API keys, chain source, node mode per network, remote-send opt-in, hsd program and data directory, explorer URL and fallback, signer session timeout) are written only by `update_protected_settings`, which validates each value, shows one old-to-new row per change in the Rust-owned secure window, and writes all changes in one transaction with an audit entry each. A URL must be https, or http to this machine, with no user name or password and its host spelled as the app reads it, so the row names the host in use. When a new host would receive a node API key the user did not type in that change, the window adds a warning naming the host. A renderer cannot choose the program the backend runs: `node_status` and `start_hsd` re-validate the stored `hsd_path` (absolute, regular, executable, canonical, writable only by its owner, owned by root or the app's user) and refuse it otherwise. Paths with control or invisible characters and network paths are refused. A send to a node whose URL host is not this machine needs the remote-send opt-in whatever the chain source label says, and the write-capability checks ask the same question. Residual risk: a compromised webview can still open the confirmation and hope the user accepts it, a confirmed node URL receives the stored API key, values written before this change are not re-confirmed or re-validated (except `hsd_path`, which is re-validated before each run), and writable parent directories of the hsd program are not checked.
- Deleting a wallet and signing a message confirm in the secure window. A delete shows label, network, kind and fingerprint; a wallet with a stored recovery phrase also shows a warning and requires its label to be typed, and the backend checks the typed text. Signing a message with a name shows the name, the signing address and the full message; the backend refuses a message over 4096 bytes, with an invisible character (control or format characters such as bidi overrides and zero-width characters), or with a long run of blank space, before anything is shown. Look-alike letters are not detected. Every confirmation: when the rows do not fit, Confirm stays disabled until they have been scrolled to the end.
- A new wallet is saved only after the user confirms, in the reveal window, that the recovery phrase is written down. A copied recovery phrase is cleared from the clipboard after about 30 seconds, or when Done is pressed, if the clipboard still holds it. Closing the reveal window with its close button cancels the timer, so the phrase stays in the clipboard until something else replaces it.
- The webview CSP has no loopback `connect-src`, `form-action 'none'` and `worker-src 'none'`, and every webview may navigate only to the app's own pages (`security.rs::navigation_guard`), so a compromised page cannot reach a node on a loopback port by request or send data out in a URL. Residual risk: it can still hold an unlocked session for up to one hour after unlock through commands that use the session without a prompt (Shakedex lock search, restore, listing imports).
- Every HNS amount the user types (Send, Bid, Lockup, Batch bid, Recover bid, Namebase withdrawal) is read by one exact parser, `src/lib/hnsAmount.ts`: plain digits, at most 6 decimals, above 0, at most the 2.04 billion HNS money supply, converted through BigInt. An amount that does not fit is refused with a sentence under the field and is never rounded. The doos validated are the doos sent. Batch Bid and Activity state the bid rule: the lockup is locked until reveal, and a bid not revealed in the reveal window forfeits its entire lockup. The Rust secure window (`review.rs::BID_REVEAL_WARNING`) remains the authoritative confirmation.

#### 2. Local disk read (malware, laptop theft, backup snapshot)

**Whole data directory:** `~/.namehold` is created owner-only (0700) and the database and its `-wal`/`-shm` files 0600, and an existing install is tightened at the next start (`db::connection::{secure_data_dir, open}`), so another account on the machine cannot read the encrypted seeds, the account xpubs, the sealed bids' values and nonces or the node api-key. `PRAGMA secure_delete` zero-fills deleted content and deleting a wallet checkpoints the write-ahead log, so a deleted wallet's encrypted seed does not stay in the file's free pages or its log (`connection::tests::{a_new_data_dir_and_database_are_owner_only, an_existing_world_readable_data_dir_and_database_are_tightened, a_deleted_secret_is_not_left_in_the_file_or_its_log}`). A file mode does not stop a program running as the same user, or a reader of an unencrypted disk or backup: for those, the seed's passphrase and full-disk encryption (FileVault, BitLocker, LUKS) are the protection.

**Namebase cookie:**

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
- Renderer cannot write the setting: `namebase_base_url` is not on `RENDERER_SETTING_KEYS`

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

Before treating `syncd.pid` as the live daemon or signalling it, the app verifies the process runs `namehold-syncd` (macOS `proc_pidpath`; Linux `/proc/<pid>/exe`, or `/proc/<pid>/comm` when that is unreadable). A reused pid is stale and gets no signal. On Windows a live pid is still taken as it is. SIGTERM runs the daemon's cleanup.

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

**Guarantee:** The background daemon never signs or broadcasts; it publishes listings already signed to LearnHNS Market. It never has access to key material. Even if the daemon is compromised, it cannot steal funds or sign malicious transactions: it can read the node (and the explorer), write sync data and publish to LearnHNS Market what the app already signed.

The "never broadcasts" half is a runtime rule, not a property of the build: the daemon runs the same `run_sync_steps` as the app, and the Shakedex purchase refresh in it holds the one path that can send (a purchase's single rebroadcast, already signed by the app). That path is closed for the daemon by `Rebroadcast::Never`: `daemon_sync_makes_no_send_call_where_the_apps_sync_does` runs the daemon's own `sync_profile` against a mock hsd holding a purchase due for its rebroadcast and checks that no `sendrawtransaction` reaches it, while the app's sync of the same wallet sends one; `daemon_never_rebroadcasts_and_leaves_it_to_the_app` checks the refresh itself under `Rebroadcast::Never`. Signing stays impossible in the daemon: it has no key material.

Publishing (R23, R25, R28) is the one outbound write it makes besides the database, on a mainnet profile only: `shakedex_jobs.rs::{publish_listings_step, advance_reverse_auctions_step, keep_listed_step}` upload a published listing's current step (a reverse auction's again whenever its schedule moves) from the listing file Finalize & sign (or Lower price) wrote and the app signed, after re-verifying every step over the lock coin hsd reports; post the day-0 pending listing; and report a mined cancel or sale. They hold no key and name no signing call, nor does the market client they write through (`shakedex_layering_tests::shakedex_jobs_hold_no_signing_call`, a lexical scan of `shakedex_jobs.rs` and `market/learnhns.rs`), and `shakedex_jobs_tests::daemon_publishes_but_never_signs_or_broadcasts` runs `run_sync_steps` as the daemon and as the app and checks the upload of the stored step, no `sendrawtransaction` and no draft created or changed, and `daemon_advances_a_reverse_auction_but_never_signs_or_broadcasts` does the same for a reverse auction whose step moved. Each write goes through `market/learnhns.rs::LearnHnsClient`, which refuses it unless the client was given the profile's network and it is mainnet; the jobs build no client off mainnet.

---

## The node the app starts, and the clients that talk to nodes

The hsd the app starts always has an api-key. It is the configured key, or a random key generated for that start and stored in the app's database, redacted from the renderer and not writable by it. It reaches hsd through `HSD_API_KEY`, never the command line. The started node listens on 127.0.0.1 whatever `hsd.conf` says. A start is refused while something that did not answer as a usable node holds the network's RPC port. A generated key is sent only to that network's default RPC port on 127.0.0.1 or localhost, and only while the node the app started is running, judged by the pid and start time recorded at spawn. It is never sent during the adoption probe of an outside node, or to a URL typed for "Test connection". Where a process start time cannot be read (Windows), no key is generated and a configured key is required to start a node. hsd's wallet plugin stays on: its HTTP requires the same key unless `hsw.conf` sets its own; an `hsw.conf` with no-auth can still turn it off. The started hsd runs with umask 077 and log level info; `namehold-hsd.log` is 0600 and has no size cap.

The node and explorer clients follow no redirects and cap replies (node 64 MiB, explorer 16 MiB). A loopback node URL ignores system and environment proxies, and localhost connects to 127.0.0.1 whatever the resolver says. A send is refused unless the node names the wallet's chain: a failed probe, or a reply with no chain, refuses it. Reads keep the tri-state rule. A node updates the wallet's coins only if it reports itself synced and, on main and testnet, its tip's median time is no more than 12 hours old (hsd's own `maxTipAge`). A node less than about 12 hours behind still counts as current.

**Residual risks:** Release builds of the app and the daemon write no core dump (`RLIMIT_CORE` 0; Linux `PR_SET_DUMPABLE` 0). The key is still in ordinary heap memory, and swap and hibernation remain a risk. A process that takes the node's port after the start's check and before hsd's bind can receive that start's generated key. On 127.0.0.1 hsd then fails to bind and exits, and the next start has a new key. On macOS, a listener on 0.0.0.0 taken in that window does not stop hsd's bind, so that node would run with a key the process heard. The window is hsd's store-opening time and is not closed. If `hsd.conf` sets `http-port`, hsd binds that port while the check and the key rule use the default port.

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

1. **Explorer trust**: In SPV mode, the wallet trusts the explorer for balance and name data. If the explorer is compromised, it could show incorrect data. Mitigation: configure a trusted explorer and optionally set a fallback URL. The explorer also says which addresses were used, and the sync derives and stores more addresses from that answer: an explorer that lists a transaction for every address makes each sync derive at most five gap limits of addresses per branch (`EXPLORER_WINDOW_ROUNDS`). That growth derives public keys and addresses only; the sync never signs, sends or touches a private key. Each of those addresses is queried on its own, so the explorer learns up to `EXPLORER_WINDOW_ROUNDS` × gap limit freshly derived addresses of this wallet per branch per sync, on top of the ones it was already asked about, all from the device's IP address.

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

**Mitigation:** Nothing the market says is trusted for the transaction's validity. Its `fee_info` sets only the market fee line's default and warning: a fee to its published address within its published rate is pre-ticked, and never above 5% of the price (`purchase::MAX_MARKET_FEE_BPS`), whatever rate it publishes. A larger fee starts unticked with its own warning, and if paid anyway that warning is shown in the secure window. Every listing is checked against the profile's own node before Buy is offered and again when the purchase is built: the lock coin must exist, hold this name and match the listing's lock address, and every price step's signature must verify. Just before broadcast the price is checked again, and the purchase is refused, and its draft discarded, if a cheaper step has become valid since it was reviewed, the step it pays is no longer valid (the median time went back, as in a reorg or on a node behind), or the node cannot say (no median time, no answer): the user reviews and signs it again. The one automatic rebroadcast of a purchase that went missing checks it the same way: at a dropped price the purchase is not sent again, and is given up with nothing paid. SPV and Explorer profiles cannot buy, because they have no node to check against.

#### 2. Network redirect or an oversized reply

**Mitigation:** The client talks only to the LearnHNS host over HTTPS and follows no redirects. Replies are capped (8 MiB for market pages, 256 KiB for a listing file). The base URL override (`learnhns_base_url`) is read only in debug builds, accepts only the LearnHNS host or loopback, and the renderer cannot write it (it is not on `RENDERER_SETTING_KEYS`).

#### 3. Signing an input the wallet does not own

**Mitigation:** The seller's lock coin is a foreign input carrying its own witness; the wallet signs only its own inputs. A Ledger signs every input as the wallet's own P2WPKH, so a plan with a foreign input, a custom sequence or a lock time is refused twice: when the draft is signed (`commands::tx`) and in the Ledger signer itself (`providers::ledger::signing`), which sees only the plan's shape. Draft signing refuses every Shakedex draft for a Ledger by its action as a class (`shakedex::is_shakedex_action`), because the FINALIZE into a lock is an ordinary plan the device signer cannot tell from any other; a plan with a lock-key or foreign input is refused by its shape. The wallet itself spends a lock coin of ours only by a cancel, signed with the lock key, which the signer derives from the seed at that moment and never stores; a buyer spends it with a price step the lock key signed at Finalize & sign (5). Locking a name (day 0) derives the name's lock key from the unlocked seed as well, only to read its public key and run the R18 self-check, whose test signature is over a template with a zero outpoint and is checked and dropped; only the public key is stored on the listing, and the TRANSFER commits to the program of the key just checked. The signer refuses an input that is both foreign and lock-key (`sign_plan_tx`), and refuses a lock-key input unless it is a `0x83` (`ANYONECANPAY|SINGLE`) input with a final sequence in a plan with lock time 0, from a receive-branch path at an unhardened index, whose output at the same index is a TRANSFER at that key's lock address with exactly 4 items, of this name (item 0 is the name hash), to a version-0 address (item 2 is `00`) whose hash (item 3) is our own receive address re-derived from the input's path (`noncustodial/shakedex/cancel.rs::check_lock_key_input`).

#### 4. The daemon and purchases

**Guarantee:** The background daemon refreshes each purchase's state from the chain but never rebroadcasts one. A purchase that went missing is rebroadcast at most once, and only by the app's own sync, through the same broadcast gates as every other send.

#### 5. Price steps signed by the lock key (selling)

**Mitigation:** The lock key signs a price step (`0x84`, `SINGLEREVERSE|ANYONECANPAY`) only in `commands/shakedex.rs::finalize_and_sign_confirmed`, behind the `shakedex_finalize_and_sign` command, and in `lower_price_confirmed`, behind `shakedex_lower_price`, each only after the user confirms its own prompt in the secure window. Before the prompt it makes one `0x84` signature, the R18 self-check's (`sell::lock_self_check`), over a test template with a zero outpoint: it is verified and dropped, never stored or sent. The prompt is `commands/secure_confirm.rs::confirm_rows`, on `prompt_secure`: its own `confirm` request, not the transaction prompt, listing the FINALIZE's network fee, every step with its price and when it becomes valid, the payment address, the lock address and the warning that each signature lets anyone buy at its price until a cancel is mined. A declined prompt signs and writes nothing. Before it asks, `prepare_lock_finalize` refuses unless the listing is the active profile's and ReadyToFinalize, the node reports the name still held, unrevoked, by the lock TRANSFER after its lockup, and that confirmed TRANSFER commits to the lock of the key re-derived from the seed for the name, whose public key must be the one stored on the listing (R18; the key is derived after every node read, held in memory until the command returns, never stored). Each step's template has one input, the lock coin the FINALIZE into the lock creates (that FINALIZE's txid, which Handshake fixes before signing, and the index of its one FINALIZE output paying the lock address, `sell::lock_output`), with that output's value, and one output paying the price to the listing's reserved payment address; its lock time is the node's median time minus 512 seconds for a Buy Now and for a reverse auction's first step, each next step of the schedule its interval later (`sell::reverse_schedule`, 2 to 1000 steps at least 15 minutes apart), read from the node just before the prompt (with the other node reads, not after it), and nothing is signed when the node does not report a median time. `sell::sign_step` signs that template with the lock key and refuses a signature that does not verify against it (sighash `0x84`, low-S, the lock key's public key). The FINALIZE and every step are signed in one hold of the unlocked session; the signed FINALIZE draft, the steps, the listing file and the listing's move to Finalizing are written in one database transaction, and nothing is sent: `commands/tx.rs::broadcast_tx_draft` sends the FINALIZE through the usual broadcast gates. A step over a FINALIZE that is never mined can never be spent on another one. A FINALIZE rebuilt at the same tip from the same coins and fee rate is byte-identical, with the same txid, so every FINALIZE txid that steps were stored over, signed here or adopted from a Restored lock's own listing file, is recorded for good (`shakedex_signed_lock_txids`) and is never signed over again: Finalize & sign refuses it before the prompt and again where the steps are stored, and the user retries after the next block. The record deliberately outlives a deleted wallet, so a re-imported seed stays protected; it holds public lock txids and listing ids, no keys. The prompt names the lock coin. Step signatures do not reach the webview: every command returns price and lock time only (`ListingStep`). They leave the wallet only in the listing file and the market upload, both once the FINALIZE into the lock is mined. `shakedex_export_listing_file` refuses to hand out the listing file while the listing is Locking, ReadyToFinalize or Finalizing, that is before its FINALIZE is mined, and for a listing that is over (Aborted, Expired, Cancelled, Sold) or Restored. Lower price asks first that the listing be Listed, with signed steps here and the active profile's, the node show its lock coin unspent, mined, a FINALIZE of the name at the lock address of the stored key and of the name's live registration (the same checks the cancel makes, `lock_on_node`), the new price be below the current step at the node's median time, and the re-derived lock key's public key be the stored one; its prompt shows the current and the new price, when the new one is valid, for a reverse auction how many later steps of its schedule still lower the price as they become valid and how many never do (walked in lock-time order, `sell::steps_ahead`), the payment address, the lock address and the warning that the earlier, dearer signatures stay valid; it signs one step over the stored lock coin paying the listing's payment address, verified before it is kept, and stores it with the rewritten listing file, read back by the strict parser, only while the listing is still Listed on that lock coin with the steps it read. Nothing is sent. Restore a lock by name (R32, `shakedex_restore_lock`) derives the lock key from the unlocked session only to read its public key and lock address, after the node reads; it signs nothing. Importing a Restored lock's own listing file (R32, `shakedex_import_own_listing_file`) derives it the same way, after the lock coin is read, only to compare its public key with the file's; it signs nothing either. The lock search (R34, `shakedex_find_locks`) derives each candidate's lock key from the unlocked session twice. The first derivation, after the node reads of the indexes, the address histories and the name by its hash and before `getnameinfo`, only reads the key's lock program to compare it with the commitment of the candidate's TRANSFER, and the key is dropped at once. When they match, the key the lock is adopted with through R32's `restore_lock_inner` is derived again after the node reads of the name's owner and its coin, R32's order, and must give the same program. It signs nothing, never stores the key, and the daemon, which holds no seed, never runs it. The lock key signs a cancel (`0x83`) only in `commands/shakedex.rs::cancel_listing_confirmed`, behind `shakedex_cancel_listing`, after the user confirms the cancel's own prompt in the secure window (the fee, the current price, where the name comes home to, that a buyer can still take the current price (and a step cheaper than the current one still ahead, of a reverse auction's schedule or a lowered price, once it is valid) until the cancel is mined, and that a purchase already in the node's mempool beats it, and for a published listing that LearnHNS Market is told the listing is cancelled at the next sync after the cancel is mined). Before it asks, `prepare_cancel` refuses unless the listing is the active profile's and Listed or Restored, the node shows its lock coin unspent, mined and a FINALIZE of the name at the lock address of the stored key, of the name's live registration, the cancel address is the profile's own receive address at the stored index under its account (`queries::receive_address_at`), the re-derived lock key's public key is the stored one, and the plan is for that account and path, its lock input marked for this name's lock key and spending the stored lock coin (`cancel::check_cancel_plan`); the signer then re-derives the destination from the input's path and refuses any other TRANSFER (`check_lock_key_input`). The signed cancel and the listing's move to Cancelling are written in one database transaction, and nothing is sent.

#### 6. Publishing to LearnHNS Market (selling)

**Scope:** Only a listing whose "Publish on LearnHNS Market" box was ticked, on a mainnet profile. `market/learnhns.rs` refuses every write (`upload_proof`, `post_pending_listing`, `refresh_status`) unless the client was given the profile's network and it is mainnet (`learnhns_tests::no_http_off_mainnet`), and the jobs make no market call at all off mainnet (`shakedex_jobs_tests::{nothing_published_off_mainnet, nothing_kept_off_mainnet}`). Publishing makes the name, the price and the lock public, and the market sees the device's IP address on every upload, post and hourly check.

**Day 0:** The pending listing (the name, the lock TRANSFER's outpoint, the lock address and the mode; no price) is posted once the lock TRANSFER's draft is `broadcast_pending`, `broadcasted` or `confirmed`, or the listing is already ReadyToFinalize or Finalizing (`db/queries.rs::list_listings_to_announce`). `broadcast_pending` is a send whose outcome the transport left unknown, so with the box ticked a pending listing can be posted for a TRANSFER the node never took. LearnHNS Market has no endpoint to withdraw a pending listing; it reads the row's status from the chain, and nothing is sent to it when the transfer is aborted.

**Uploads are unauthenticated:** `POST /api/upload-proof` takes no credential, so anyone holding a valid copy of our listing, or any other proof for the name, can upload it in place of ours. Keeping the listing on the market (R25, `shakedex_jobs.rs::keep_listed_step`) is the only defence: about hourly the market's copy (`GET /listing/<name>/proof.json`) is compared with the step we would upload (`listing_file::same_market_listing`: name, lock outpoint, public key, payment address, one step with the same price, lock time and signature, no fee address in effect, the same `expiresAt`), and ours is uploaded again unless they match. A copy naming a fee address is treated as someone else's, because `0x84` does not commit to a fee output.

**Only the market's own answer is a verdict:** a reply counts only when its body is the market's JSON shape; an HTML page, a redirect, a 5xx, a timeout or a body without the market's fields is no answer. No answer, and the market's own reply that it has not seen it on chain yet, are retried with backoff (5 minutes doubling to 6 hours); the market's own refusal is kept with its words and not retried until what would be sent changes. A mined cancel or sale is reported (R28) once the market has taken our pending post or an upload of this listing, whatever later uploads got (`market_told`, which nothing clears), and never for a listing the market took nothing of; the market's own refusal of that report is kept with its words and ends the reporting (`db/queries.rs::list_listings_to_report`, `shakedex_jobs.rs::report`).

**Nothing unverified is uploaded:** before an upload every stored step is re-verified over the lock coin hsd reports, at the value it reports (`shakedex_jobs.rs::market_copy`); a step that does not verify is never uploaded and the listing is marked steps unverified (`shakedex_jobs_tests::steps_not_signed_by_the_lock_are_not_uploaded`). A reverse auction's upload is its current step alone (R24), the same re-verification first. The jobs hold no key and sign nothing (4 in "Background sync daemon").

---

## Mitigations reference table

| Concern | Mitigation | Location | Tests |
|---------|-----------|----------|-------|
| Renderer reads cookie | Redacted; presence marker only | `settings.rs:12-16` | `settings_cmd_tests` |
| Renderer writes cookie | `update_setting` allowlist (`RENDERER_SETTING_KEYS`) | `security.rs` | `settings_cmd_tests` |
| Renderer writes base URL | `update_setting` allowlist (`RENDERER_SETTING_KEYS`) | `security.rs` | `settings_cmd_tests` |
| Renderer changes the hsd program, node, explorer or an API key | `update_setting` refuses them; written only after a secure-window confirmation; the stored program is re-validated before each run | `security.rs` (`PROTECTED_SETTING_KEYS`), `protected_settings.rs` | `protected_settings_tests`, `settings_cmd_tests::update_setting_refuses_every_protected_key_and_leaves_its_row` |
| Wallet deleted or message signed without confirmation | Secure window; a phrase-holding wallet needs its label typed; a message with hidden text is refused | `commands/secure_wallet.rs`, `noncustodial/message.rs` | `secure_wallet_cmd_tests`, `sign_name_message_tests` |
| Confirm pressed on rows never seen | Confirm stays disabled until overflowing rows are scrolled to the end | `src/secure/render.ts` | `src/secure/__tests__/render.test.ts` |
| Generated node key reaches another process | New key per start, sent only to the app's own running node on the network's default port; a held port refuses the start | `noncustodial/rpc.rs` (`managed_node_api_key_for_url`), `commands/node.rs`, `noncustodial/node_config.rs` | `node_lifecycle_tests` |
| Stale node decides which coins are spent | Coins are updated only from a synced node with a recent tip | `noncustodial/rpc.rs`, `commands/sync.rs` | `sync_race_tests::{node_step_against_a_node_with_an_old_tip_marks_nothing_spent, node_step_against_a_node_behind_the_tip_marks_nothing_spent}` |
| Lock coin signed over twice | Every FINALIZE txid with stored steps is recorded and refused for good | `commands/shakedex.rs`, `db/queries.rs` (`shakedex_signed_lock_txids`) | `shakedex_sell_tests::{a_resigned_finalize_never_reuses_a_lock_coin_steps_were_signed_over, a_finalize_the_sync_reverted_stays_refused_at_the_same_tip, deleting_the_wallet_keeps_the_signed_lock_record, finalize_and_sign_returns_no_step_signatures}`, `shakedex_my_listings_tests::my_listings_carry_no_step_signatures` |
| Base URL redirect (production) | Release build ignores setting | `namebase.rs:24-27` | `namebase.rs::tests` |
| Cookie on disk (offline attacker) | AES-256-GCM under OS-keyring DEK | `cookie_vault.rs` | `cookie_vault::tests` |
| Signing without confirmation | Rust-owned secure window | `tx.rs::sign_tx_draft_confirmed` (`prompt_secure`) | `tx_lifecycle_tests` |
| Signing something other than what the window showed | Rows of a name action computed from its plan; a send signs only its reserved coins; change re-derived; signed txid must equal the reviewed one; a revoke needs the name typed | `noncustodial/review.rs` (`covenant_rows`), `commands/tx.rs` (`ensure_reviewed`, `ensure_signed_is_reviewed`) | `review_tests`, `tx_lifecycle_tests::{a_max_send_never_sweeps_a_coin_that_arrived_after_it_was_reviewed, a_send_whose_reservation_was_released_is_refused_before_the_window_opens, signing_refuses_a_send_whose_change_is_not_the_wallets_change_address, signing_refuses_a_name_plan_whose_change_is_not_ours, a_revoke_is_signed_only_once_its_name_is_typed_in_the_window, the_window_shows_a_bids_value_from_the_wallets_record}` |
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
| Daemon crashes mid-sync | Heartbeat every 10s; stale-lock takeover after 30s; app respawns daemon on next startup | `db/sync_lock.rs`, `commands/daemon_ctl.rs` | `sync_lock` tests |
| Concurrent writes by app + daemon | Cross-process `sync_locks` table; app acquires with priority, daemon preempts stale locks; a run whose lock another process took over (its heartbeat finds the row is no longer its own) stops before its next step, address-window round or write, and ends on `lock_lost`; the market jobs also read the lock row before each listing, so no market call waits for the next heartbeat | `db/sync_lock.rs` (`LockLost`), `commands/sync.rs` (`spawn_lock_heartbeat`, `run_sync_steps`), `shakedex_jobs.rs` (`market_lock_lost`), `daemon/mod.rs` | `sync_lock`, `sync_race` tests, `sync_race_tests::lock_lost_tests::{heartbeat_raises_lock_lost_once_another_process_takes_the_lock, run_sync_steps_stops_once_the_lock_is_lost_mid_sync}`, `address_window_tests::a_lost_lock_stops_the_window_before_its_next_round`, `shakedex_jobs_tests::market_jobs_stop_before_a_listing_once_the_sync_lock_is_lost` (every check site, the flag and another process's row) |
| Daemon signs (would-be) | Daemon has no access to key material — it reads hsd, writes sync data and, on mainnet, publishes listings the app already signed; the market jobs name no signing call | `bin/namehold-syncd.rs`, `daemon/mod.rs`, `shakedex_jobs.rs` | (no key material in the daemon process), `shakedex_layering_tests::shakedex_jobs_hold_no_signing_call` |
| Daemon broadcasts (would-be) | The one send path in `run_sync_steps` (a purchase's rebroadcast) is closed for the daemon at runtime by `Rebroadcast::Never` | `commands/sync.rs`, `shakedex_jobs.rs` | `shakedex_purchase_state_tests::{daemon_sync_makes_no_send_call_where_the_apps_sync_does, daemon_never_rebroadcasts_and_leaves_it_to_the_app}`, `shakedex_jobs_tests::{daemon_publishes_but_never_signs_or_broadcasts, daemon_advances_a_reverse_auction_but_never_signs_or_broadcasts}` |
| Daemon publishes to the market | Mainnet only; already-signed steps re-verified on hsd; no key in the jobs | `market/learnhns.rs`, `shakedex_jobs.rs` | `learnhns_tests::no_http_off_mainnet`, `shakedex_jobs_tests::{daemon_publishes_but_never_signs_or_broadcasts, daemon_advances_a_reverse_auction_but_never_signs_or_broadcasts, steps_not_signed_by_the_lock_are_not_uploaded}` |
| hsd left running after app exit | Intentional when "Sync in background" ON; hsd bound to loopback + api-key required | `lib.rs` (setup/exit hooks) | `settings-background-sync` tests |

---

## Release and update trust

Updates are trusted through one Ed25519 updater key; its public half is in `src-tauri/tauri.conf.json`. Whoever can use that key can ship an update every install accepts, after a user clicks Install. Key rotation is not implemented, so a stolen key cannot be retired today. The key is held as a GitHub Actions secret, and only the sign job names it.

A release runs four jobs. `prepare` refuses a commit that is not in main's history or lacks green Frontend, Rust lint, Rust tests, Workflow Validation and E2E checks, and refuses a version already published. Any older commit of main with green CI passes, so someone with write access can re-release old code. `build` compiles with no key and no write token, and restores nothing from the Actions cache. `sign` is the only job that holds the key; it runs in the `release` environment, has no write token, and runs only GitHub's checkout and artifact actions and the Tauri CLI, installed from a lockfile with install scripts off. A compromised build can still get a bad bundle signed, because `sign` signs what `build` produced, but it cannot read the key. `publish` verifies every signature against the key in `tauri.conf.json` for this version, requires each installer kind exactly once and a matching `SHA256SUMS`, writes `latest.json`, attests provenance, and requires the draft to match by name and digest before it un-drafts.

`scripts/check-supply-chain.mjs` enforces the job list, permissions, guard steps, SHA pins and the place of the signing secrets. It reads workflows line by line and refuses YAML it could misread (flow mappings, quoted or tagged keys, double-quoted escapes, anchors, aliases, merge keys). That check catches mistakes in review. It does not stop a workflow pushed to a branch, which runs before review, or someone with write access, who can edit the check in the same commit.

Each release carries `SHA256SUMS` and a `.sig` for every file, the dmg included. They use the updater key, online in the workflow, so they show a file matches what the workflow signed and do not protect against someone who holds that key or can run the sign job. macOS and Windows builds are not code-signed.

---

## Secrets in the repository

`scripts/check-secrets.mjs` refuses BIP39 recovery phrases with a valid checksum, written with any common separator or decoration (lists, tables, comments, HTML, joined by one of `-._/:`, JSON arrays and objects), also inside base64; extended private keys of all four Handshake networks and WIF keys; and 16, 32 or 64 bytes of hex near the words seed, entropy, priv, secret, master or mnemonic. It runs before a commit or merge commit on staged blobs, and before a push on every commit the remote lacks, including file names, messages, authors, annotated tags and ref names; a ref to a blob or tree is refused. The `Secret scan` workflow runs on every branch, tag and pull request on the tree and the pushed commits. Hooks are client-side and can be bypassed with `--no-verify`; CI detects only after publication, when the secret must be treated as burned. Not detected: hex as byte arrays, keys or hex split across lines, secrets inside binary files, base64 with a non-base64 prefix glued to it such as a path. A cherry-pick runs no commit hook and is covered only at push. Public vectors are allowed by SHA-256 in `scripts/secret-scan-allow.txt`.

---

## Dependency auditing

CI runs an advisory-only audit job (`cargo audit` + `pnpm audit --prod`) on
every PR to surface newly-disclosed CVEs in the dependency graph. The vector generator's lockfile is tracked, and developer harnesses install with `--ignore-scripts`, except leveldown for the shakedex CLI.

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
| [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367), [-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368), [-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369), [-0429](https://rustsec.org/advisories/RUSTSEC-2026-0429) | `zip 4.6.1` | vuln (extraction: symlinks, permission bits, a process abort on a symlink entry with a large declared size) | Fixed only in `>=9.0.0` (semver-major). Pinned by `tauri-plugin-updater` (even `2.14.0` depends on `zip 4.6.1`). Extraction runs only in the Windows updater, on a bundle whose Ed25519 signature `download_and_install` has already verified against `plugins.updater.pubkey`: a malicious archive needs our signing key (see "Release and update trust" for where it lives). Revisit when the updater moves to zip `>=9`. |
| [RUSTSEC-2026-0370](https://rustsec.org/advisories/RUSTSEC-2026-0370), [-0371](https://rustsec.org/advisories/RUSTSEC-2026-0371), [-0372](https://rustsec.org/advisories/RUSTSEC-2026-0372) | `zvariant`, `zbus_names`, `zvariant_derive` (4.x and 5.x) | unmaintained | Pulled in by `zbus` (Linux D-Bus) via `notify-rust`, `tauri-plugin-notification` and `tauri-plugin-opener`. No fixed release; revisit when zbus moves off them. |

> `anyhow` (RUSTSEC-2026-0190) was fixed by bumping to `1.0.104` rather than
> suppressed, since it had a semver-compatible patch. `rustls` (RUSTSEC-2026-0346 to -0349) and `quinn-proto`
> (RUSTSEC-2026-0338 to -0342) were fixed the same way, by `0.23.46` and `0.11.19`.

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
