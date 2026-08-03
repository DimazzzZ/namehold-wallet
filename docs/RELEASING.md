# Releasing Namehold

## Prerequisites (one-time setup)

### 1. Generate the updater signing keypair

The auto-updater requires an Ed25519 keypair. The **public key** is embedded in
the app binary (via `tauri.conf.json`); the **private key** signs every release
bundle so existing installs can verify the update came from you.

```bash
pnpm tauri signer generate -w ~/.tauri/namehold.key
```

This creates:
- `~/.tauri/namehold.key` — **PRIVATE** key. Back it up offline (1Password /
  hardware / paper). **If you lose this key, no future update can reach existing
  installs.** You would have to ask every user to reinstall from scratch.
- `~/.tauri/namehold.key.pub` — public key (safe to share).

### 2. Wire the keys

1. Paste the contents of `namehold.key.pub` into
   `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`.
2. Add two GitHub Actions secrets to the repo:
   - `TAURI_SIGNING_PRIVATE_KEY` — the full contents of `~/.tauri/namehold.key`.
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password you chose (or empty).

---

## Cutting a release

1. Bump the version in `src-tauri/tauri.conf.json`, `package.json`,
   `src-tauri/Cargo.toml`, `src/components/Layout.tsx`, and
   `src/lib/webqa-mock.ts` (`current_version`). Run `cargo check` in
   `src-tauri/` to update `Cargo.lock`. Add a CHANGELOG entry: rename
   `[Unreleased]` → `[X.Y.Z] - <date>` and open a fresh `[Unreleased]`.
2. Commit and tag:
   ```bash
   git tag v0.X.0
   git push origin main --tags
   ```
3. The `release.yml` workflow:
   - Creates a **draft** GitHub Release.
   - Builds on macOS (universal), Linux x86_64, Linux ARM64, and Windows.
   - Runs `beforeBuildCommand` to build the `namehold-syncd` sidecar binary
     per-platform (macOS universal, both Linux architectures, Windows) and
     stages it for bundling.
   - `tauri-action` produces the bundles **+ `.sig` files + `latest.json`**
     (because `createUpdaterArtifacts: true` and the signing key is set).
   - The sidecar binary is bundled into each app bundle via the `externalBin`
     configuration in `tauri.conf.json`.
   - Verifies `latest.json` contains signed updater entries for macOS x86_64 /
     ARM64, Linux x86_64 / ARM64, and Windows x86_64.
   - Un-drafts the release (makes it public + "latest").
4. Existing installs auto-detect the new version within ~30 s of their next
   launch (or immediately when the user clicks "Check for updates" in Settings).

---

## How the updater works

- Endpoint: `https://github.com/DimazzzZ/namehold-wallet/releases/latest/download/latest.json`
- `latest.json` contains the new version, per-platform download URLs, and the
  Ed25519 signature of each bundle.
- The plugin verifies the signature against the embedded public key before
  installing. If the signature doesn't match, the update is rejected.
- On Windows the app exits automatically during install (OS limitation). On
  macOS/Linux the user clicks "Restart now" (or the app restarts itself).

---

## Key rotation (if ever needed)

The plugin supports setting the pubkey at runtime via `UpdaterBuilder::pubkey()`.
To rotate: ship a release signed with the OLD key that embeds the NEW pubkey,
then all subsequent releases use the new key. This is not implemented yet — the
current setup uses a single static key.
