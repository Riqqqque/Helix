# Dashboard and Home

This page covers the Helix dashboard itself: the sidebar and top bar, every
page, the Home layout editor, themes and colors, how preferences sync, and the
Settings page. Game servers have their own guide in
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace).

## Layout of the dashboard

| Area | What it holds |
| --- | --- |
| Sidebar | The page list, an **Arrange** button, **Settings** pinned at the bottom, a connection dot, and a **Help** link to this wiki. |
| Top bar | Host name and current page, the time of the last update, the bell menu, a refresh button, the theme menu, and your account menu with **Sign out**. |
| Background activity | A tray that lists running jobs (installs, backups, updates) with their stage and progress. It is safe to reload; Helix reattaches to the same jobs. Finished jobs can be dismissed. |

Each page has its own address after `#`, for example `#storage` or
`#servers`. An empty fragment opens Home, so a bookmarked dashboard lands on
Home. Pages load on first use; hovering a sidebar link preloads it.

### The bell menu

The bell lists notices, such as a mount over 90% full. **Open** goes to the
related page. Closing a notice hides it in this browser only. Other browsers
and other people still see it. **Settings → Helix data → Dismissed notices →
Show them again** brings hidden notices back.

### Refreshing

Host data refreshes every five seconds by default. Change it under **Settings
→ Live data**: 1, 2, 5, 10, or 30 seconds. Polling pauses while the tab is
hidden. The top-bar refresh button asks for a fresh snapshot right away.

## Pages

| Page | Address | What it shows | Needs |
| --- | --- | --- | --- |
| Overview | `#overview` | Live host summary: CPU, memory, Helix's own footprint (dashboard containers and broker, games excluded), storage, servers, services, network, and every Docker container. An optional live-graphs toggle. | signed in |
| Home | `#home` | Your own grid of widgets. See [Home](#home). | signed in |
| Storage | `#storage` | Drive cards, the space analyzer, and the file manager. See [Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files). | `storage.files.manage` to change files |
| Network | `#network` | Interfaces (Docker bridges and loopback folded away), the routing table, listeners, Docker port bindings, UFW, and game port pools. See [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates). | `network.firewall.write` to change rules |
| Host | `#host` | Host facts, Linux package updates, **Update Helix**, systemd services, top processes, and Docker containers. | `system.power` for reboots |
| Security | `#security` | Host hardening, firewall and SSH facts, recommendations, and the few Helix switches that need a typed confirmation. See [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery). | `system.settings.write` to change |
| Terminal | `#terminal` | A real Linux shell as one non-root user. See [Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal). | `terminal.open` |
| Machines | `#machines` | Other computers on your rack: probes, Wake-on-LAN, reboot, power off, SSH. See [Machines](#machines). | `machines.view`, `machines.manage` |
| Servers | `#servers` | Native and imported game servers. | `games.manage` to change |
| Hooks | `#hooks` | Connected services such as Plex, Jellyfin, Tailscale, Docker, and AMP. | `system.settings.write` to act |
| Strands | `#strands` | Installed extensions. See [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands). | `system.settings.write` to install |
| Globe | `#globe` | A world map of this host and its connections. Hidden until you add it. See [Globe](#globe). | signed in |
| Settings | `#settings` | Dashboard and account settings. See [Settings](#settings). | signed in |

### Arranging pages

Choose **Arrange** beside **Pages** in the sidebar, or use **Settings →
Navigation**, to:

- move a page up or down;
- hide a page; or
- add a hidden page back.

Hidden pages keep their place, so adding one puts it back where it was.
Settings always stays at the bottom. Hiding **Servers** turns off the game
server dashboard but leaves running servers alone. **Reset pages** restores
the factory order with Globe hidden.

## Home

### First Home

A new owner starts with one Home called **Main**: a clock, weather, host pulse,
servers, storage, and a **Scratchpad** note. Home greets you by the display
name from owner setup. You can type a city into the weather widget and write
in the note without entering layout mode. Nothing is pre-filled with a fake
city or demo server.

### Widgets

| Widget | Shows | Minimum size (columns × rows) |
| --- | --- | --- |
| Clock | The time and date. | 2 × 2 |
| Host pulse | CPU, memory, load, uptime. | 2 × 3 |
| Live graphs | CPU, memory, and load drawn from samples kept in this browser. Helix has no timeseries database. | 4 × 4 |
| Servers | Your game servers and their state. | 2 × 3 |
| Storage | Mounts and free space. | 2 × 3 |
| Docker | Every container on this host. | 2 × 3 |
| Weather | A five-day forecast for a city or postal code, in Celsius or Fahrenheit. | 3 × 3 |
| Notes | Up to eight named pages of text. | 2 × 3 |
| Shortcut | A link to any `http://` or `https://` address, opened in a new tab. | 1 × 2 |
| Strand | An installed, enabled Strand that declared `helix:ui.widget`. | 2 × 3 |
| Globe | The same country-level map as the Globe page. | 4 × 5 |

Weather comes from Open-Meteo through `helixd`, which caches each answer for
ten minutes. Your browser does not contact Open-Meteo directly.

Shortcut icons are fetched by your browser from the public
[dashboard-icons](https://github.com/homarr-labs/dashboard-icons) set on
`cdn.jsdelivr.net`, matched by the icon name, the app name, the host name, or
the port. A shortcut can also use its own `http(s)` icon address. If no icon
loads, the tile shows the first letter of its name.

### Editing the layout

Choose **Edit layout**. Home is a 12-column grid with 52-pixel rows.

- **Move:** drag a widget by its title bar or the grip in its toolbar. A
  dashed outline shows where it will land, and other widgets move out of the
  way. Widgets settle upward so there are no gaps above them.
- **Resize:** drag the bottom-right corner. Sizes snap to whole columns and
  rows, and each widget has the minimum size listed above.
- **Add widget:** opens the list of widget types.
- **Settings (gear):** title, exact width and height, accent color (or
  **Use Home color**), and **Copy to another Home**. Notes also have **Quick
  editing**; Globe has **Data motion**.
- **Remove:** in the widget toolbar.
- **Done editing** saves and leaves layout mode.

### Copy and paste

| Action | How |
| --- | --- |
| Copy one widget | Select it, then **Copy** in its toolbar or Ctrl+C (Cmd+C on macOS). |
| Copy every widget on this Home | **Copy all**, or Ctrl+A. Ctrl+C with nothing selected does the same. |
| Paste here | **Paste**, or Ctrl+V. Pasted widgets go into the first free spot. |
| Paste onto another Home | **Paste onto another Home**, then pick the Home. |

On a plain HTTP dashboard the browser often blocks the system clipboard.
Helix then pastes from its own saved copy in this browser.

### Import from Homarr

While editing, **Import from Homarr** reads Homarr apps on this host that
already have an `http(s)` address. Current Homarr keeps them in SQLite; older
JSON configs also work. Tick the apps you want and choose **Add N to
<Home>**. They become ordinary Shortcut tiles on the Home you are editing, in
Homarr's order. Re-importing skips shortcuts already on that Home. Homarr
media uploads, notes, and Homarr-only widgets stay in Homarr.

### Several Homes

Choose **Homes** to:

- switch between Homes;
- rename the current Home or change its accent color;
- create a **New blank** Home or **Duplicate** this one;
- **Export** this Home as a `.helix-home.json` file;
- **Import** a Home file (up to 70 KB); or
- **Delete Home** after a second confirmation (**Keep it** cancels).

| Limit | Value |
| --- | --- |
| Homes | 8 |
| Widgets per Home | 32 |
| Widgets across all Homes | 64 |
| Note pages per note | 8 |
| Characters per note page | 7,000 |

At least one Home always remains. Imported files are checked before anything
changes.

### Full screen

**Full screen** hides the sidebar and top bar. Choose **Exit full screen** on
the Home page to bring them back.

### Notes

Notes are stored with your other preferences. They are not an encrypted vault.
Addresses and reminders are fine; keep passwords, keys, tokens, and recovery
codes in a password manager.

## Globe

Globe draws this host from its router's public country, then a pin for each
country with established public TCP connections. Connections to game ports
Helix knows (native and AMP) are game-port lines, including pings and join
attempts. Other public connections are outbound lines. Loopback, LAN, CGNAT,
and overlay addresses never appear.

The country lookup happens in the broker. Your browser never receives remote
IP addresses. Lines are solid by default. **Data motion** animates them;
faster dots mean more sessions or queued traffic toward that country, not a
bandwidth meter. Systems set to reduce motion keep solid lines. Country data
is the public NRO whois table (CC0); land outlines are Natural Earth 110m.

## Machines

Machines tracks other computers, such as boxes on a rack, from this host.

1. Choose how Helix signs in:
   - **Helix hub key**: the broker keeps its own SSH keypair and known-hosts
     file. Add the shown public key to the machine's `authorized_keys`.
     Probes, Wake, and power all work.
   - **Password prompt**: the SSH password prompt appears in the terminal.
     Nothing is stored, so probes and power actions are unavailable.
   - **Host SSH config**: uses the terminal user's own `~/.ssh` keys and
     config.
2. Enter a **Label**, **Host or IP**, **SSH port**, **SSH username**, and
   optionally a **Wake-on-LAN MAC** and **Notes**.

Each machine card shows SSH latency, host name, kernel, uptime, load, CPU
cores, memory, failed units, containers, and the busiest processes from its
last probe. Actions are **Reboot**, **Power off**, Wake-on-LAN, and **SSH**.
SSH sessions run inside the same non-root terminal service as the Terminal
page. **Remove machine** forgets the entry and cached probe data; remove the
hub key from the machine yourself.

## Settings

Settings opens with a sync badge: **Dashboard preferences are synced**,
**Saving…**, **Loading…**, or **Preference service unavailable · using the
browser copy**. The cards are:

| Card | What you can do |
| --- | --- |
| Live data | Pick the refresh interval. |
| Game servers | Show or hide the Servers page. Hiding it does not stop any server. |
| Catalogs | Save or remove your CurseForge API key. It is kept in a private file on this host and never shown again. CurseForge also needs this host on a normal ISP address; VPS and VPN exits are often blocked. Modrinth needs no key. |
| Server API tokens | Create, view (after your password), rotate, and revoke tokens for scripts and tools. See [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens). |
| Appearance | Theme and the color control center. See [Themes and colors](#themes-and-colors). |
| Navigation | Order, hide, add, or reset sidebar pages. |
| Helix data | Counts of native servers, imported connections, and removed servers. Per native server: **Start after the host boots**. **Recoverable trash** to restore or delete removed servers. **Forgotten AMP connections** to show them on Servers again. **Dismissed notices** with **Show them again**. |
| Host integration | Docker and broker service state, the dashboard and gateway container restart policies, and **Helix dashboard after boot**. |
| Whole-host reboot | Restart Linux now or on a recurring schedule. A safety check blocks it while players are online or jobs are running. |
| Docker cleanup | Measure Docker's data folder and remove only build cache, dangling images, and unused networks older than the chosen age (one week by default), now or on a schedule. Containers, volumes, and named images stay. |
| Signed-in session | **Stay signed in** on or off. Off means 30 minutes idle or 8 hours total. |
| Owner account | Change the login name, display name, or password. Needs your current password and signs out every session. |

**Start after the host boots** on a native server means the game comes back
after Linux or Docker restarts. It is on by default for new servers and does
not start or stop anything right now. It is separate from **Helix dashboard
after boot**, which only covers the Helix containers.

Removed servers in **Recoverable trash** can be restored. **Delete forever**
erases the world files, their Helix backups, and console history. It is not an
off-host backup.

## Themes and colors

The theme menu in the top bar and **Settings → Appearance** offer:

| Theme | Look |
| --- | --- |
| System | Follows this device: Light or Midnight. |
| Midnight | The default dark palette. |
| OLED | True black surfaces. |
| Light | Light palette. |

The color control center overrides **Accent**, **Text**, and **Panels** with
any hex color, each with its own reset. Only `#rrggbb` values are accepted.

The theme is saved in this browser only. The three colors sync with your other
preferences.

## How preferences sync

Page order, hidden pages, the refresh interval, colors, the Servers toggle,
and every Home layout are saved in Helix's state database, so they follow your
account across browsers and are included in state backups.

- Each save sends the expected revision. If another browser saved first,
  Helix fetches the newer copy, merges only the fields you changed, and saves
  again.
- If the save cannot reach Helix, this browser keeps the change, shows
  **Using this browser's saved copy**, and retries.
- The Home page shows the same status: **Layout synced through Helix**,
  **Saving layout…**, or the browser-copy warning.

Some things stay in one browser only: the theme, dismissed notices, the
Overview graphs toggle, Home full screen, terminal text size, and forgotten AMP
connections.

## Keyboard and accessibility

- **Skip to content** is the first focusable link on every page.
- The sidebar and mobile bar mark the current page for screen readers.
- Home editing: click a widget to select it, arrows move it one cell,
  Shift+arrows resize it, Escape cancels a drag. Shortcuts are ignored while
  you type in a field.
- Dialogs keep focus inside until closed.
- Background activity is announced politely as it changes.
- Systems set to reduce motion get no animated Globe lines.

## Phones and narrow windows

- At 900 pixels wide or less, the sidebar is replaced by a bottom bar with the
  same pages plus Settings. It scrolls sideways when there are many pages.
- When the Home grid is narrower than 720 pixels, widgets stack in one column
  in reading order. Settings, copy, and remove still work; widen the window to
  move or resize.

## Related pages

- [Getting Started](https://github.com/Riqqqque/Helix/wiki/Getting-Started)
- [Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files)
- [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands)
- [Glossary](https://github.com/Riqqqque/Helix/wiki/Glossary)
