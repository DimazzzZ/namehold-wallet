# Icon assets

## Brand mark

- `logo.png` / `icon.png` — the full-color 790×790 Namehold logo, shown on
  the About page and used as the app/bundle icon.

## Tray (menu bar) icons

Three-state monochrome template icons rendered on macOS in the menu bar and
on Windows/Linux in the tray:

- `tray-normal.png` — filled logo silhouette (running & synced)
- `tray-syncing.png` — filled silhouette + activity dot bottom-right
- `tray-stopped.png` — outlined (hollow) silhouette (node stopped)

All three are 44×44 pure black on transparent, i.e. **template images**. On
macOS the tray builder calls `set_icon_as_template(true)` so AppKit inverts
the glyph automatically to match the menu bar (light or dark).

### Regenerating

If `logo.png` is ever redesigned, regenerate the three tray PNGs from its
alpha channel by running the generator script:

```sh
python3 src-tauri/icons/gen-tray-icons.py
```

It reads `logo.png`, tight-crops to the mark, and emits the three
templates. There is no separate SVG source — the logo's alpha is the
authoritative silhouette.
