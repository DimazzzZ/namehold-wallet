# App knowledge for QA agents

Where things are in Namehold, how its windows behave under automation, the words it uses for a listing's states, and the issues already known. Exact sentences a case checks are quoted in that case; this file holds what many cases share.

## Where things are

- **Sidebar**, top to bottom: Wallet, Activity, Auctions, Market (tagged "New"), Watchlist, Move from Namebase, Settings. Hotkeys follow the order: 4 opens Market, 5 Watchlist, 6 Move from Namebase, 7 Settings.
- **Wallet page**: the profile selector in the header; `Signer unlocked` / `Signer locked` with `Lock` and `Unlock`; the **Receive Address** (`rs1…` on regtest; `View all addresses` lists the derived window); `Sync`; **Owned Names** with a checkbox per row, the batch buttons (`Renew Selected`, `Transfer Selected`, …) and `Manage` per row.
- **Owned Names row of a listed name**: its listing state label (the same words as My listings, below) and a `My listings` button that opens Market › My listings.
- **Owned Names row of a purchase on its way**: the name as plain text (no link), no checkbox, no `Manage`; badge `Unconfirmed purchase` → `Awaiting finalize · N blocks` (`1 block`) → `Ready to finalize` with a `Finalize` button → `Finalize · waiting for a block` → an ordinary owned row.
- **Name actions** open from `Manage`, from the name itself (the `.name` link, "View name info"), or from Auctions after looking a name up. The Ownership section holds `Transfer`, `Finalize`, `Cancel transfer`, `Sell…`, `Renew`, `Revoke`. A disabled button's reason is a hover hint.
- **Market**: title `Market`, subtitle "Names their owners are selling through Shakedex. Your node checks every listing before you can buy."; `Import listing…` on the right (Browse only); underlined tabs `Browse` and `My listings`. Browse, top to bottom: on mainnet with the Shakedex setting off the banner "Buying on mainnet is off. …" with `Turn on buying`; the `Your purchases` card while a purchase is not finalized (`Paid <price> HNS`, a progress bar with `<done> of <lockup> blocks` or "Ready to finalize", `~<time> left` on mainnet and testnet (`<n> blocks left` on regtest) or `Finalize`, filled only while the page shows no other filled button: not beside the banner or a listing's Buy); the list (toolbar `Find a name`, `All` / `Fixed price` / `Price drops`, sort `Lowest price` / `Next price drop` / `Newest`; columns Name · Price now · Next price · Check; rows `✓ Verified`, or `Couldn't check` with `Retry`; at its foot the "unavailable listings hidden" toggle and `Load more` or "All LearnHNS listings loaded") and the listing panel beside it (below it in a narrow window) with the price, `Checked by your node`, the breakdown, `▸ Fee rate`, the three steps Buy · `Wait ~2 days` (`Wait 10 blocks` on regtest) · Finalize, and one full-width button; the panel is never taller than the window from where it starts, so with a long list its button stays in sight and the rest scrolls inside it. Off mainnet the list reads "Nothing is listed right now" / "There is no LearnHNS Market on this network. If a seller sent you a listing directly, import it." `Import listing…` opens `Import a listing`: a box for the file's text or a link, `Choose a file…`, `Cancel`, and `Import` (or `Import link` for text starting with `http(s)://`; `Checking…` while the node checks it). Imports join the list, tagged `From file` or `From link`. My listings lists the active profile's listings and holds `Restore a lock by name`.
- **Activity**: a search box ("Search by name..."), an action filter (`All actions`, then one entry per action) and a status filter (`All statuses`, `Confirmed`, `Pending`). Draft rows offer `Sign & broadcast` (unsigned), `Broadcast` (signed), `Retry` (failed or dropped) and `Discard`. A row that moves a name shows its value on a `⤷ <value>` line.
- **Settings**: Connections (`Allow sending via remote node`, `Sync in background (keep wallet up to date when app is closed)`, `Save settings`); Notifications (`Enable deadline notifications`, which also drives the Shakedex "Ready to finalize" reminders); Shakedex ("Allow buying and selling names on mainnet through Shakedex (experimental)", with the note "Testnet and regtest are always allowed. Finalizing a purchase, and Finalize & sign, Cancel and Lower price on a listing you already have, work whatever this says.").

## Amounts

Amounts are written with six decimals everywhere: `5.000000 HNS`, `0.050000 HNS`. A case that writes `5 HNS` in a quoted UI string means `5.000000 HNS`. A backend refusal shown in the UI may carry the prefix "Invalid input: " or "Broadcast failed: "; that prefix is fine.

## Secure windows and native panels

- Signing happens in a **secure window**, a separate window of the same process. When the signer is locked, the first secure window asks for the wallet passphrase (`Wallet passphrase`); after it, the action's own secure window opens (for example `Confirm cancel`, `Confirm Finalize & sign`, `Confirm Lower price`, `Confirm transaction`) and is confirmed with `Confirm & Sign` or declined with `Cancel`. Declining shows "Confirmation declined" in the dialog that asked.
- Find secure windows again after every action that opens one; they may open behind the main window or on another macOS Space.
- The native **Save** and **Open** panels (export, own-file import, `choose a file…`) are separate windows, possibly of another process (the macOS panel service). Type the path into the name field, or use `cmd+shift+g` (Go to folder).
- If the human maintainer watches the run, a secure window may be confirmed by them by hand; note who confirmed each signature.

## Driving the app

Any GUI driver works: computer use, or an accessibility driver such as cua-driver (`list_windows`, `get_window_state`, element actions by snapshot index). Lessons learned with this app (WebKit inside Tauri):

- Re-snapshot before every element action; indices change on every render. An element action needs the snapshot it came from.
- A background press by element index usually works and may report its effect as unverifiable; verify with a fresh snapshot or a screenshot.
- An HTML `<select>` (for example the onboarding network dropdown) has no accessibility children and cannot be set by value; focus it and press `down` (foreground) until the option shows, then `return`.
- If typing into a text field by element index does not show up in the next snapshot, click the field by window-local pixel coordinates and type; as a last resort use foreground delivery.
- Foreground actions briefly raise the app window; keep them to what is needed.
- Hover hints (a disabled button's reason) are often not exposed to accessibility. A case that needs the hover text and cannot read it is ◐, not ❌.
- Toasts are transient and easy to miss between two snapshots. A missed toast is ◐ with the reason, never ❌; when it matters, snapshot right after the confirm.

## Listing states

The same label on Owned Names and on the My listings badge (`src/components/market/marketText.ts`). Which buttons a state shows in My listings is fixed; whether each is enabled is the backend's, and every disabled button's reason is printed once under the row.

| State | Label | My listings buttons |
|---|---|---|
| locking | `Locking`, or `Locking · N blocks` once the lock is mined | Finalize & sign |
| readyToFinalize | `Ready to Finalize & sign` | Finalize & sign |
| finalizing | `Finalizing into the lock` | none |
| listed | `Listed for sale` | Lower price, Cancel listing, Export listing file (Publish on LearnHNS Market: mainnet only) |
| salePending | `Sale pending` | Export listing file |
| sold | `Sold` | none |
| cancelling | `Cancelling` | Lower price, Export listing file |
| cancelAwaitingFinalize | `Cancelled · finalize to bring it home` | Finalize home, Export listing file |
| cancelFinalizing | `Coming home` | Export listing file |
| cancelled | `Cancelled` | none |
| aborted | `Lock aborted` | none |
| restored | `Locked — listing details unknown` | Cancel listing, Import its listing file |
| expired | `Expired in the lock` | none |

- A row with its own unsent transaction also shows `Sign and send` (unsigned) or `Send` (signed), and `Discard`; a transaction the node may already hold shows `Send` and no `Discard`.
- The mode next to the name is `Buy now` or `Reverse auction`; a Restored lock shows none.
- The price line shows **only on Listed rows**: Buy Now `Price 5.000000 HNS`; reverse auction `Now X HNS · next Y HNS in ~T · floor Z HNS` (or `No price step valid yet · …`); node unreachable `Current price could not be checked: <reason>`. A Listed row with more than one signed step (a reverse auction, or a Buy Now after a Lower price) also has the disclosure `All N price steps`, one line `<price> HNS — from <date time>` per step; that is correct for a lowered Buy Now too. Under it: `Listed until <date> · stays buyable until sold or cancelled`. Wait texts read `~N min`, `~N h`, `~N d`.
- Off mainnet every row's market status line reads `Shared as a file only: LearnHNS Market lists mainnet names only`.

## Activity labels for Shakedex

A Shakedex draft is labelled by what it does, and keeps that label after its transaction is seen on chain: `Buy`, `Finalize purchase`, `Lock for sale`, `Finalize into lock`, `Cancel listing`, `Finalize cancel`. A row this device has no draft for (made on another device or profile of the same phrase) shows the chain's generic label (`Transfer`, `Finalize`); that is not a finding.

Amounts on those rows: a purchase shows minus the price (not the name's value); a seller's cancel and a buyer's FINALIZE show 0 with a `⤷ <name value>` line; the fee is shown separately. A lost purchase carries the badge `Lost` with its reason under it.

## Known quirks and their workarounds

- **The window may open on another macOS Space**; switch to it before capturing.
- **Transient toasts and hover hints** are easy to miss: record ◐ with what was seen, not ❌.
- **Quitting with a secure window open**: `cmd+Q` may not close the app while a secure window is up; use the app menu's Quit, or `osascript -e 'tell application "Namehold" to quit'`, and as a last resort `pkill -x namehold-wallet`.

## Known issues NOT to report again

These are recorded and parked. Note what you see if a case runs into one, but file it again only if its behaviour changed.

- A name in the Cancelling state shows "Shakedex_cancel is broadcast and waiting to be mined. Nothing changes on-chain until it lands in a block." in its name actions instead of disabled owner buttons.
- Spec §5 "Known gaps" ([the spec](../specs/2026-10-05-shakedex-name-sales.md)) lists behaviour that is by design (one device per listing, a restore sees addresses only up to the gap limit past the last used one, and so on): a case that hits one is ◐ with the gap named, not a bug.
