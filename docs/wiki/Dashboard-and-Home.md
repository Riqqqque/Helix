# Dashboard and Home

## First Home

A new owner starts with clock, weather, host, servers, storage, and a scratchpad
note. Home greets the display name from owner setup. Type a city on the weather
widget without entering layout mode, write in the note, then use **Edit layout**
for shortcuts and arrangement. Nothing is pre-filled with a fake city or demo
server.

## Navigation and refresh

Overview, Home, Storage, Network, Host, Security, Terminal, Servers, Hooks,
Strands, and Globe are full pages. Globe starts hidden; add it from **Arrange**
or Settings. An empty URL fragment opens Home so a pinned tab lands on the
dashboard. Settings stays at the bottom of the sidebar. Choose **Arrange**
beside Pages to move, hide, or add primary pages; the set and order follow the
owner account. Hidden pages keep their slot, so Add puts them back where they
were. A refresh does not snap pages back to the factory order. Helix saves the
layout to this host, and if you reload before that save lands, this browser
keeps the change and retries.

Host metrics refresh every five seconds by default. Change that interval in
**Settings → Dashboard behavior**. The top-right refresh button requests a
fresh snapshot immediately and stays static between requests. A saved one-second
interval stays one second.

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

### Arranging widgets

Choose **Edit layout**. Home is a 12-column grid, and every widget keeps exactly
the same size while you edit.

- **Move:** drag a widget by its title bar (or the grip in its toolbar). It
  follows your cursor, a dashed outline shows where it will land, and the other
  widgets move out of the way as you go. Drop it anywhere — beside, above, or
  under other widgets, including in the space under a short widget next to a
  tall one. Widgets settle upward so there are no empty holes above them.
- **Resize:** drag the corner at a widget's bottom right. It snaps to whole
  columns and rows, and each widget has a sensible minimum size.
- **Keyboard:** click a widget to select it, then use the arrow keys to nudge it
  one cell, or Shift and the arrow keys to resize it. Escape cancels a drag.
- **Settings:** the gear in a widget's toolbar opens its settings above the grid
  — title, exact width and height, accent color, and **Copy to another Home**.

Click a tile to select it, then **Copy** (or Ctrl+C / Cmd+C) and **Paste** (or
Ctrl+V / Cmd+V) to duplicate it on this Home or another one; pasted widgets find
the first free spot. Positions are saved with the layout and follow you between
browsers. On narrow windows, Home stacks widgets in one column in reading order;
widen the window to move or resize them.

Available widgets are Clock, Host pulse, Live graphs, Servers, Storage, Docker,
Weather, Notes, Shortcut, Strand, and Globe. Shortcuts accept only validated HTTP(S)
URLs and open in a new tab. Strand widgets embed an enabled package that
declared `helix:ui.widget`. The Globe widget is the same country-level map as
the Globe page, including an optional data-motion toggle in widget settings. While editing, **Import from Homarr** reads Homarr
apps that already have an http(s) address. Current Homarr stores those in SQLite; older
Homarr JSON configs still work. Helix adds the checked shortcuts onto **the Home
you are already editing**, in Homarr's layout order as square tiles. They do not
create a separate Homarr Home. Those tiles are normal widgets: rename them,
resize them, recolor them, or copy them onto any other Home. Re-import skips
shortcuts that are already on that Home. On a plain HTTP dashboard, Paste uses Helix's
saved copy when the browser blocks the system clipboard. Long names stay inside
the picker rows and tiles. Icons come
from Homarr's http(s) icon URLs, Homarr icon names via the dashboard-icons set, or
Helix's matcher on the app name and link. Uploaded Homarr media files, notes, and
Homarr-only apps stay in Homarr.

**Copy all** copies every widget on the current Home. The button reads **Copied**
for a moment after it works. **Paste onto another Home**
drops that whole set onto the destination in one go, then you can rearrange.
Ctrl/Cmd+A selects all tiles while editing; Ctrl/Cmd+C copies the selection, or
all tiles if nothing is selected. Widget settings still has **Copy to another
Home** for a single tile. **Full screen**
hides the sidebar and the top bar (Helix, refresh, theme, account). Exit full
screen from the Home page button to bring them back.

Overview can show the same live graphs behind a toggle. Those samples live in
the browser; Helix does not keep a timeseries database.

## Globe

Globe is off the sidebar until you add it. It draws this host from the router's
public WAN country, then country pins for established public TCP peers. Game
ports Helix already knows (native and AMP) count as game-port connections,
including pings and join attempts; everything else public is outbound. Loopback,
LAN, CGNAT, and overlay addresses stay off the map. Lookup happens in
helix-privd; the browser never receives remote IPs. The Home widget map fills
its card. The Globe page keeps the full world visible.

Lines default to solid. **Data motion** is opt-in. Dots travel faster where more
sessions or queued traffic share a country. That is not a bytes-per-second
meter. Reduced-motion systems stay on solid lines. Country data comes from the
public NRO whois country table (CC0); land outlines are Natural Earth 110m.

## Notes

A Notes widget can hold up to eight named pages. Its settings control whether
the active page can be edited without entering layout mode. Changes use the same
preference save as the rest of Home. If you reload before Helix accepts the save,
this browser keeps the notes and retries.

Notes are application preferences, not an encrypted secret vault. Addresses and
ordinary reminders are reasonable; passwords, private keys, access tokens, and
recovery codes belong in a password manager.

## Themes and colors

The built-in themes are System, Midnight, OLED, and Light. Settings also exposes
bounded accent, surface, and text controls with previews and a reset action.
Helix validates contrast-sensitive values rather than accepting arbitrary CSS.

## Dismissed notices

If you close a full-disk warning on Overview, a notice in the bell menu, or the
Storage space-analyzer intro, Helix remembers that in this browser only. Other
browsers and other people on this dashboard still see those banners. The disk is
unchanged. **Settings → Helix data → Show them again** brings the banners back.

## Helix data

Settings → Helix data lists native servers Helix owns, imported connections it
can see, recoverable trash, and AMP connections this browser has forgotten.
Recoverable trash can restore a removed native server or delete it forever.
Each native server has **Start after the host
boots**: on means the game comes back after Linux or Docker restarts; off means
it stays stopped until you press Start. That same checkbox is on by default
when you create a server. It does not start or stop the server right now. This
is not the Host integration toggle for the Helix dashboard containers.
