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
2. Add two secrets to the `release` environment of the repository (Settings → Environments), not to the repository's Actions secrets. Only the `sign` job of `release.yml` declares that environment, so with a required reviewer and deployments limited to `main` and `v*` tags no other workflow or branch can read them. The workflows cannot create these rules: until they are set in the repository's settings, the environment has none and repository-level secrets still reach every workflow.
   - `TAURI_SIGNING_PRIVATE_KEY` — the full contents of `~/.tauri/namehold.key`.
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password you chose (or empty).
3. Move the key off the machine you develop on once it is in the environment and backed up. Code that runs during a build or a test can read anything in the home directory.

`scripts/check-supply-chain.mjs` fails Workflow Validation if a workflow names either secret outside the signing step of `release.yml`'s `sign` job or reaches secrets without naming one. That catches a mistake in review; a workflow pushed to a branch still runs before any review, so the environment is what actually keeps the key out of reach.

---

## Cutting a release

1. Bump the version:
   ```bash
   pnpm version:set X.Y.Z
   ```
   This updates `package.json`, `src-tauri/tauri.conf.json`,
   `e2e/package.json` and `src-tauri/Cargo.toml` in one shot, then runs `cargo check` to refresh
   `Cargo.lock`. The frontend derives its version from these at build time
   (backend via `CARGO_PKG_VERSION`, webqa mock via a Vite `define`), so no
   `.tsx` file needs editing.

   Then add a CHANGELOG entry: rename `[Unreleased]` → `[X.Y.Z] - <date>` and
   open a fresh `[Unreleased]`.
2. Refresh the README screenshots, so they show this version's screens and version number:
   ```bash
   just screenshots
   ```
   Look over the changed images in `docs/assets/` (`git diff --stat docs/assets` lists them) and commit them with the version bump. A screen that changed shape may need its showcase data or scene updated; see [screenshots/README.md](../screenshots/README.md).
3. Refresh the list of strings applied for in ICANN's 2026 gTLD round (the ICANN badge on names), and commit it if it changed:
   ```bash
   just icann-strings
   ```
   The script prints the counts it wrote and refuses an export whose layout changed. ICANN confirms the final list on 17 November 2026 and applicants may still withdraw after that, so this step is not optional for a release after that date.
4. Merge to `main`, wait for CI on that commit to go green, then tag it:
   ```bash
   git tag v0.X.0
   git push origin v0.X.0
   ```
   A manual run of `release.yml` from `main` works the same way. The commit must be on `main` and its Frontend, Rust lint, Rust tests, Workflow Validation and E2E checks must have passed; otherwise the run stops before it opens a draft (`scripts/release-gate.mjs`). A tag pushed before CI finishes fails this way too: re-run the job once CI is green. The advisory dependency audit is not part of this. Any older commit of `main` with green CI also passes, so someone with write access can re-release old code under a new version; the gate does not check that the commit is the newest one.
5. The `release.yml` workflow runs four jobs:
   - **prepare** checks the commit as above and creates a **draft** GitHub Release.
   - **build** compiles on macOS (universal), Linux and Windows with no signing key and no write token, because `tauri build` runs every crate's `build.rs` and proc macros and the npm build tools. It restores nothing from the Actions cache, so a release compiles from scratch and takes longer than a CI run. `pnpm tauri build --no-sign` still writes the updater archives; `scripts/release-files.mjs stage` hands the installers on as artifacts. `--no-sign` would also skip macOS and Windows code signing, so adding code signing means revisiting that flag.
   - **sign** is the only job with the key, and it waits for the `release` environment's approval. It runs GitHub's artifact actions and the Tauri CLI, installed from `scripts/release-signer/package-lock.json` with install scripts off. It writes `SHA256SUMS` and a `.sig` for every file, the dmg and `SHA256SUMS` included, each naming the version. It has no write token. A compromised build can still get a bad bundle signed here, but it cannot take the key.
   - **publish** holds a write token and no key. It refuses the release unless each of the six installer kinds is there exactly once, nothing unknown is there, every signature verifies against the key in `tauri.conf.json` for this version, and `SHA256SUMS` matches (`release-files.mjs check`). It writes `latest.json` from those signatures, attests build provenance, uploads everything, checks that the draft holds exactly those files by GitHub's digest (`release-files.mjs compare`), and un-drafts.

   `SHA256SUMS` and the signatures use the updater key, so they show a file matches what this workflow signed; they do not protect against someone who holds that key or can run the `sign` job. `scripts/check-supply-chain.mjs` keeps these jobs, their permissions and their guard steps as written: changing a guard step means changing its copy in that script in the same commit.
6. Existing installs auto-detect the new version within ~30 s of their next
   launch (or immediately when the user clicks "Check for updates" in Settings).

---

## Pinned actions and toolchain

Every `uses:` names a commit SHA (`scripts/check-supply-chain.mjs` refuses anything else) and Dependabot proposes the bumps weekly. These it does not bump, so they are done by hand: `dtolnay/rust-toolchain`, pinned to a commit of its `stable` branch; its `toolchain:` input and `rust-toolchain.toml`, which must be the same version in every workflow; the `--version` of `cargo install tauri-driver` and `cargo install cargo-audit`; and the Tauri CLI in `scripts/release-signer`, which must be a version `pnpm-lock.yaml` also pins (`npm install --package-lock-only --ignore-scripts` there after changing it).

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
