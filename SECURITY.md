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

---

## The Namebase migration feature

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
  (`commands/tx.rs:505-506`) showing action, recipient, amount, fee, txid, and
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

## Mitigations reference table

| Concern | Mitigation | Location | Tests |
|---------|-----------|----------|-------|
| Renderer reads cookie | Redacted; presence marker only | `settings.rs:12-16` | `settings_cmd_tests` |
| Renderer writes cookie | `RENDERER_WRITE_DENYLIST` | `security.rs:20` | `settings_cmd_tests` |
| Renderer writes base URL | `RENDERER_WRITE_DENYLIST` | `security.rs:20` | `settings_cmd_tests` |
| Base URL redirect (production) | Release build ignores setting | `namebase.rs:24-27` | `namebase.rs::tests` |
| Cookie on disk (offline attacker) | AES-256-GCM under OS-keyring DEK | `cookie_vault.rs` | `cookie_vault::tests` |
| Signing without confirmation | Rust-owned secure window | `tx.rs:505-506` | `tx_lifecycle_tests` |
| RPC api-key sent cleartext | `guard_transport` rejects remote HTTP | `rpc.rs:139-168` | `rpc.rs::tests` |
| Audit log leaks secrets | Redacted to `***` on write; re-redacted on read | `settings.rs:40-41, 68-69` | `settings_cmd_tests` |

---

## Dependency auditing

CI runs an advisory-only audit job (`cargo audit` + `pnpm audit --prod`) on
every PR to surface newly-disclosed CVEs in the dependency graph.

### Suppressed advisories

A suppressed advisory is one we have reviewed and determined does not apply to
this app. Suppressions live in `package.json` under
`pnpm.auditConfig.ignoreGhsas` and MUST be justified here.

| Advisory | Package | Rationale |
|----------|---------|-----------|
| [GHSA-qwww-vcr4-c8h2](https://github.com/advisories/GHSA-qwww-vcr4-c8h2) | `react-router` | CSRF bypass that the advisory states "only affects your application if you are using the unstable RSC APIs". Namehold is a Tauri single-page app with client-side routing only — it does not use React Server Components, so the vulnerable code path is never reached. Revisit when upgrading to `react-router@>=8.3.0`. |

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
