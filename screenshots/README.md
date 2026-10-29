# README screenshots

Generates the screenshots in `docs/assets/` that the main README shows. The app runs in a plain browser on its browser mock (`src/lib/webqa-mock.ts`), a showcase wallet is laid over the mock's own data, and each scene is framed as a macOS window.

```bash
just screenshots                  # every scene
just screenshots market names     # only these
```

The first run needs Playwright's browser: `cd screenshots && pnpm exec playwright install chromium`.

On one machine, two runs give the same bytes: the data, the page clock (2026-10-23 12:00 UTC), the locale (en-US) and the time zone (UTC) are all fixed, so an image changes only when its screen did. Another OS or Chromium version renders fonts a little differently, so shoot a release's set on one machine. Nothing is written until every scene is shot, so a failing scene leaves the previous set whole.

## Layout

- `scenes.ts` lists the scenes: the file written to `docs/assets/<file>.png`, the route, and what to click or scroll before the shot. A new screenshot is one entry here.
- `showcase/world.ts` is the showcase wallet: profile, balances, owned names, auctions in each phase, purchases, renewals and the imported Namebase history. It is typed against `src/types`, so `pnpm typecheck` fails when a backend shape changes under it. Next-action words and countdown labels are copied from the backend (`next_action_for_task` in `commands/names.rs`, `extract_countdown` in `commands/names_pure.rs`), not from the mock.
- `showcase/index.ts` installs the world through the mock's `globalThis.__webqa_overrides` hook: a command listed there is answered from the showcase (a value, or a function of the call's args), everything else keeps the mock's answer. The Browse tab's listings are the mock's own demo listings; the rows of My listings are the showcase's, each taking its actions and refusal sentences from the backend's fixture `src/test/fixtures/listing-actions.json` for its state.
- `frame.ts` draws the window around the shot; `shoot.ts` starts Vite on port 5199, runs the scenes and writes the files.
- `derive-keys.py` derives the showcase addresses and xpub: well-formed, but from fixed seeds, so they belong to nobody.

## When a screen changes

`shoot.ts` prints, per scene, the commands the mock had no answer for. A new one on a scene usually means the screen reads something the showcase does not cover yet: add it to `showcase/index.ts` (and the data to `world.ts`) if it shows on the screenshot. Look over every image before committing it; the script cannot tell whether a screen still tells its story.
