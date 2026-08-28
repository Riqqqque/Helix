# Dashboard and Home

## First Home

A new owner starts with clock, weather, host, servers, storage, and a scratchpad
note. Home greets the display name from owner setup. Type a city on the weather
widget without entering layout mode, write in the note, then use **Edit layout**
for shortcuts and arrangement. Nothing is pre-filled with a fake city or demo
server.

## Navigation and refresh

Overview, Home, Storage, Network, Host, Security, Terminal, Servers, Hooks, and
Strands are full pages. An empty URL fragment opens Home so a pinned tab lands on the
dashboard. Settings stays at the bottom of the sidebar. Choose **Arrange**
beside Pages to move the primary pages up or down; the order follows the owner
account.

Host metrics refresh every second by default. Change that interval in
**Settings → Dashboard behavior**. The top-right refresh button requests a
fresh snapshot immediately and stays static between requests.

## Home layouts

Home supports several independent layouts. Open **Templates** to:

- switch the active Home;
- create a blank layout;
- duplicate or rename the current layout;
- export one layout as bounded JSON;
- import a shared Helix Home JSON file; or
- remove a layout after confirmation.

At least one Home always remains. Imported data is validated before it replaces
anything. Export is the easiest portable copy, while the authoritative layouts
are also stored in Helix's revisioned state database and included in state
backups.

Choose **Edit layout** to add widgets, drag them by their handle, or move them
with keyboard-accessible controls. Every widget can choose width, height, title,
and accent color. The grid reflows at desktop, tablet, and mobile widths, so a
layout with one widget and a layout at the configured maximum both stay usable.

Available widgets are Clock, Host pulse, Live graphs, Servers, Storage, Docker,
Weather, Notes, Shortcut, and Strand. Shortcuts accept only validated HTTP(S)
URLs and open in a new tab. Strand widgets embed an enabled package that
declared `helix:ui.widget`. While editing, **Import from Homarr** reads Homarr
apps that already have an http(s) address. Current Homarr stores those in SQLite; older
Homarr JSON configs still work. Relative icons, notes, and Homarr-only apps stay
in Homarr. Shortcuts already on this Home are left unchecked. **Full screen**
hides the sidebar and the top bar (Helix, refresh, theme, account). Exit full
screen from the Home page button to bring them back.

Overview can show the same live graphs behind a toggle. Those samples live in
the browser; Helix does not keep a timeseries database.

## Notes

A Notes widget can hold up to eight named pages. Its settings control whether
the active page can be edited without entering layout mode. Changes use the same
debounced, revision-aware preference save as the rest of Home, with a browser
fallback that retries if the server is temporarily unavailable.

Notes are application preferences, not an encrypted secret vault. Addresses and
ordinary reminders are reasonable; passwords, private keys, access tokens, and
recovery codes belong in a password manager.

## Themes and colors

The built-in themes are System, Midnight, OLED, and Light. Settings also exposes
bounded accent, surface, and text controls with previews and a reset action.
Helix validates contrast-sensitive values rather than accepting arbitrary CSS.

## Dismissed notices

Capacity notices on Overview can be dismissed from the card or from the
notifications bell in the top bar. Dismissal is this browser’s local list; it
does not change the disk, remove files, or hide the same evidence inside
Storage. Settings can show those notices again.
