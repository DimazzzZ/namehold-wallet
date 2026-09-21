# Coding Standards

How code is written in this repo. Tooling enforces formatting and lints; this
file covers what tooling cannot. A documented rule here overrides the generic
code-smell heuristics a reviewer might otherwise apply.

## Gates (must be clean before every commit)

| Layer    | Command (from)                                              |
|----------|-------------------------------------------------------------|
| Rust     | `cargo fmt --check` (`src-tauri/`)                          |
| Rust     | `cargo clippy --all-targets -- -D warnings` (`src-tauri/`)  |
| Rust     | `cargo test` (`src-tauri/`) — CI runs the same tests via `cargo nextest run --manifest-path src-tauri/Cargo.toml --locked` |
| Frontend | `npx tsc -b && npx vite build` (repo root)                  |
| Frontend | `npx vitest run` (repo root)                                |
| Frontend | `npm run lint:format` (prettier), `npm run lint:secure-imports` and `npm run lint:native-title` |

Locally `cargo test` is enough; CI uses nextest for its two-lane split (see
`src-tauri/.config/nextest.toml`).

## Rust layering

- `src-tauri/src/commands/*` are **IO shells**: they lock the DB, build clients,
  call Tauri. They import from `db::queries`, `noncustodial::*`, `providers::*`
  and shared helpers. **New** code must not import helpers from a sibling
  command module; if two commands need the same thing it belongs in
  `db::queries` (data), `noncustodial::*` (pure domain logic), or a small
  dedicated helper module under `commands/` (e.g. `commands/active_profile.rs`).
  Existing imports of `commands::read` (profile resolution, node readiness),
  `commands::sync::open_conn`, `commands::namebase` and
  `commands::secure_prompt` are legacy shared layers — do not add to them.
- **Pure logic is split from IO** so it can be unit-tested without a live node
  or a Tauri `State`: the `*_with_client(&dyn NodeRpc, ...)` pattern for RPC
  code, `*_from_conn(&Connection)` for DB code, `*_pure.rs` modules for
  larger bodies.
- `#[cfg_attr(coverage_nightly, coverage(off))]` goes **only** on IO shells
  (a `#[tauri::command]`, a function that locks `state.db`, spawns a process,
  or hits the network). Never on a pure function — write a test instead.
- `Option` means "unknown", not "false". A tri-state check such as
  `network_check(expected, reported) -> Option<bool>` returns `None` when
  either side is unknown, and callers act only on `Some(false)`. Document the
  `None` meaning at the type or function.
- Errors are `AppError`. A DB failure in a **user-triggered** command is
  returned (`?`), not swallowed into a default. Silent fallbacks
  (`.ok().flatten()`, `unwrap_or_default()`) are allowed only in routing /
  best-effort paths, and the line above them must say why.

## Rust tests

- Unit tests live in `src-tauri/src/tests/<area>_tests.rs`, registered in
  `src-tauri/src/tests/mod.rs` in alphabetical order. Tests that need private
  access to a module's internals may live in a `#[cfg(test)] mod` inside it.
- Test names state the behaviour: `check_node_connection_flags_a_cross_network_node`,
  not `test_probe_2`.
- Mock the node with `tests/mock_node_rpc.rs::MockNodeRpc`; build a DB with
  `Connection::open_in_memory()` + `crate::db::migrations::run(&conn)`.
- Tests that depend on randomness must assert on the actual value they
  produced, never on a forced one (see `cookie_vault.rs::flip_hex_digit`).
- When a helper is moved, its tests move with it. Never delete a test to make
  a refactor compile.
- **Process-global state needs a serial key.** `std::env::set_var` /
  `remove_var` change the variable for the whole process, and the harness runs
  tests as threads of one process — so a test that swaps `HOME` races every
  test that reads it, including indirectly. Put `#[serial(<key>)]`
  (`serial_test`) on the writer **and on every reader**, sharing one key per
  variable. Rust 2024 marks these functions `unsafe` for exactly this reason.
  Cost of getting it wrong: `test_hsd_candidates_includes_home_paths` failed
  about one full run in twenty and passed alone every time.
- **Wait for content, never for the file.** `fs::read_to_string(p).ok()` is
  `Some("")` the instant a file exists, and a writer that redirects (`echo x >
  f`) creates and truncates before it writes a byte. A poll loop keyed on
  `Some(_)` therefore exits on the empty window and asserts against nothing.
  Treat an empty read as "not ready" (see `node_lifecycle_tests::recorded_argv`).
- A test that passes alone and fails in the suite is a defect in the test, not
  a reason to retry it. Find the shared state; the two rules above are the two
  ways it has bitten so far.

## TypeScript ↔ Rust bridge

- A Rust struct and its TS mirror must agree field-for-field on spelling. New
  structs use `#[serde(rename_all = "camelCase")]` (e.g. `NodeConnectionCheck`);
  a few legacy models (`Asset`, `Settings`) are snake_case on both sides — do
  not "fix" them, the frontend depends on the spelling. Command **arguments**
  stay snake_case (`api_key`), matching Tauri's argument mapping.
- The TS mirror lives in `src/types/index.ts`. Adding a field means: Rust
  struct + TS interface + **every** test fixture that builds that object (grep
  the tests for a neighbouring field, e.g. `synced:`).
- Tri-state fields are `Option<bool>` ↔ `boolean | null`. No enums across the
  bridge for a three-valued flag.

## Frontend

- Behaviour shared by two screens lives in one hook (`useNodeConnectionCheck`)
  or one module (`lib/connectionMode.ts`), so the screens cannot drift on what
  a word like "connected" means.
- Untyped form state is cast **once** into a typed local at the top of the
  component, not at every use.
- Hover hints use the `Tooltip` component, never the native `title` attribute
  — `npm run lint:native-title` enforces it. A `title` cannot be styled or
  positioned, has no delay we control, and is **never shown on an element with
  `pointer-events: none`**, which every disabled `Button` sets: the hint
  disappears exactly when it has something to say. `title` remains an ordinary
  prop on `Dialog`, `PageHeader`, `Card`, `Alert` and `EmptyState` (a heading,
  not a hint) and on `Badge`, which renders a Tooltip itself.
  - The Tooltip's trigger wrapper is a real element — floating-ui measures it,
    so it cannot be `display: contents`. It defaults to `inline-flex`; pass
    `className` (which **replaces** that default) whenever the parent's layout
    depends on the trigger, e.g. a block list item or a `flex-1` / `shrink-0`
    flex child.
  - A Tooltip renders a `span`, so it cannot wrap a `<td>` or `<tr>`. Wrap the
    cell's contents instead.
  - When the `title` was also the element's accessible name (an icon with no
    text), add an `aria-label` — the Tooltip does not name the trigger.
- Components use `data-testid` for anything a test clicks or asserts on.
  Tests use Vitest + Testing Library, mock `invoke` per command name, and
  build **complete** fixtures (every field of the type, no partials).
- A test that pins a contract the app cannot currently reach (e.g. a state
  the backend never produces on that screen) says so in a comment above the
  `it(...)`.

## Copy, comments and docs

- UI text, doc comments, `CHANGELOG.md` and `docs/*.md` may only claim what
  the code **enforces**. "X will be refused" requires a code path that
  refuses X. If only reads are gated, say reads.
- A comment that points at another file (`(read.rs)`) is a sign the rule
  should be extracted and shared instead.
- Every user-visible change gets a bullet under `## [Unreleased]` in
  `CHANGELOG.md`. Node / connection behaviour is documented in
  `docs/NODE_SETUP.md`; security-relevant behaviour in `SECURITY.md`.
- Features that span backend + frontend + docs get a spec in `docs/specs/`
  (see `docs/specs/README.md`), written before or with the code, and kept
  current when behaviour changes.

## Commits

- Subject: `type: summary` with `feat` / `fix` / `refactor` / `test` / `docs`
  / `build` / `chore` / `format`. Body explains **why** and what a reviewer
  should look at; list the gates you ran when the change is non-trivial.
- One logical change per commit. Review follow-ups may be bundled when the
  message enumerates them, but a test-only refactor unrelated to the subject
  is its own commit.
- No tool-attribution trailers.

## Review

Changes are reviewed on two axes: **Standards** (this file plus the Fowler
smell baseline as judgement calls) and **Spec** (the feature's
`docs/specs/*.md`). A finding that says "either fix the code or soften the
claim" is resolved toward whichever the product intent actually is — and the
spec is updated to record the choice.
