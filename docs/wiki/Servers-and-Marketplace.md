# Servers and Marketplace

The **Servers** page creates and runs game servers on the Helix host. Each
Helix-managed server runs in its own Docker container under its own numeric user,
with its own folder, ports, memory and CPU limits, console history, and backups.
This page covers the supported games, creating servers, the server page, everyday
actions, settings and limits, ports, updates, the marketplace and modpacks,
imported AMP servers, and removing servers.

Related pages:

- [Backups](https://github.com/Riqqqque/Helix/wiki/Backups): back up, restore, keep rules, and the backup trash.
- [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes): file changes made while a server runs.
- [Hytale](https://github.com/Riqqqque/Helix/wiki/Hytale), [Pumpkin](https://github.com/Riqqqque/Helix/wiki/Pumpkin), and [Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim): game-specific guides.
- [Copy a server into Helix](https://github.com/Riqqqque/Helix/wiki/Server-Migration) and [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers).
- [Game Hosting and Capacity](https://github.com/Riqqqque/Helix/wiki/Game-Hosting-and-Capacity): sizing memory, CPU, and disk.
- [Finding and editing a Helix server](https://github.com/Riqqqque/Helix/blob/main/docs/SERVER-AUTOMATION.md): host paths and safe file replacement from SSH and scripts.

## Supported games

| Game | Software | Default port | Notes |
| --- | --- | --- | --- |
| Minecraft Java | Paper, Purpur, Folia, Leaves, Pufferfish, Fabric, Quilt, Forge, NeoForge, Vanilla, a modpack, or your own JAR | 25565 TCP | Console, settings form, marketplace, live backups |
| Minecraft (Pumpkin) | Pumpkin, a native Rust server | 25565 TCP plus a Bedrock TCP/UDP port | See [Pumpkin](https://github.com/Riqqqque/Helix/wiki/Pumpkin) |
| Hytale | Official dedicated server on Java 25 | 5520 UDP (QUIC) | See [Hytale](https://github.com/Riqqqque/Helix/wiki/Hytale) |
| Valheim | Steam dedicated server | 2456–2457 UDP | Thunderstore mods. See [Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim) |
| V Rising | Windows dedicated server under Wine | Game and query UDP | In-game server list on by default |
| Terraria | Vanilla or tModLoader | 7777 TCP | `.tmod` mods in `mods/` |
| Palworld | Steam dedicated server | 8211 UDP | Community server list on by default |
| Satisfactory | Steam dedicated server | 7777 TCP+UDP, plus a UDP beacon port | |
| Project Zomboid | Steam dedicated server | 16261 UDP, plus a Steam UDP port | Admin password generated if left blank |
| 7 Days to Die | Steam dedicated server | 26900 TCP+UDP, plus two UDP ports | |
| Rust | Steam dedicated server | 28015 UDP, plus UDP query and TCP RCON | RCON password generated |
| Sons of the Forest | Windows dedicated server under Wine | 8766 UDP, plus two UDP ports | |
| Factorio | Official headless server | 34197 UDP | Public listing needs a Factorio account token |
| Don't Starve Together | Klei dedicated server | 10999 UDP, plus one for Caves | A Klei cluster token is needed for public listing |
| Vintage Story | Official .NET dedicated server | 42420 TCP | |

The game chooser uses Helix's own marks, not publisher artwork. Nothing is
installed on the host operating system: each game's runtime is a Docker image that
Helix builds the first time you create a server for that game. When you remove the
last active server of a game that has its own runtime image (V Rising, Valheim,
Terraria, Palworld, and the Steam games), Helix deletes that image. A removed
server can still be restored; Helix rebuilds the image if needed.

The Servers list shows a filter button only for games you actually have, plus
**Helix** (native servers only) and imported connections. Native and imported
servers carry different ownership labels.

## Creating a server

Choose **New server** and pick a game. Every create dialog asks for a name,
memory, an optional CPU cap, ports (automatic from the game's port pool, or a
specific port), and whether the server should start after the host boots. While
the server is created, the window shows a spinner, a percent, and the elapsed
time. The job keeps running if you close the page.

### Minecraft

Pick the software and the Minecraft version. The version list loads published
releases for that software, including **Latest stable**. Paper, Folia, and Leaves
leave out experimental versions that create would refuse. You also choose the
player limit, private or public player access, and accept the Minecraft EULA.

- Forge uses the official installer for Minecraft 1.17 and newer and launches
  with its generated `unix_args`. The installer runs as the server's own user.
- Purpur and Pufferfish come from their publishers over HTTPS without a pinned
  checksum.
- **Use your own JAR** accepts a dropped `.jar`, the Upload picker, or an absolute
  path inside an allowed import folder. Uploads go into Helix's private import
  folder in chunks (16 KiB to 768 MiB, and the file must start like a ZIP). You
  type the exact Minecraft version and pick Java 17, 21, or 25. Helix cannot tell
  what an arbitrary JAR contains, so custom servers get no marketplace and no
  automatic updates; replace the JAR through Files after a backup. Extra import
  folders are set with `native.custom_artifact_roots`; `/` is never allowed.
- **Start with a modpack** creates a server from a Modrinth or CurseForge pack.
  See [Modpacks](#modpacks).

Memory for Minecraft is 1–24 GiB (the create dialog offers 2–16 GiB). For a
modpack, Helix suggests an amount from the loader and the host's free memory
until you choose one yourself.

### V Rising

Helix builds `helix-vrising-runtime:1` the first time and installs the server
with SteamCMD. **Show on the V Rising server list** is on by default. It writes
`ListOnEOS`, `ListOnSteam`, and `HideIPAddress` in
`save/Settings/ServerHostSettings.json`, so friends can join from the in-game
browser through EOS without a port forward. Direct Connect to your public address
still needs the UDP game and query ports forwarded. You can change listing later
on Overview; a running server needs a restart to pick it up. The first install
often takes 10–30 minutes. Player counts are not queried and there is no command
console. Memory is 2–24 GiB.

### Valheim

See [Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim). It uses two UDP
ports, the game port and the next one. Memory is 1–16 GiB.

### Terraria

Helix builds `helix-terraria-runtime:1`. Vanilla downloads the publisher's
dedicated server zip; tModLoader installs Steam app 1281930. Edit
`serverconfig.txt` in Files, drop `.tmod` files in `mods/`, and restart. Memory is
512 MiB to 8 GiB.

### Palworld

Helix installs the dedicated server from Steam. Listing on the community server
browser is on by default. Direct Connect needs the UDP game and query ports
forwarded. Palworld needs 8 GiB of memory or more to run well; the range is
4–32 GiB.

### Other Steam and dedicated games

Satisfactory, Project Zomboid, 7 Days to Die, Rust, Sons of the Forest, Factorio,
Don't Starve Together, and Vintage Story share one create flow. Helix reserves
the consecutive ports each game needs, asks for passwords, tokens, world name, or
seed where the game uses them, and offers **Show on the server list** where the
game has one. Sons of the Forest runs under Wine, which adds startup time and
memory. Each game has its own memory range in the dialog.

## The server page

Open a server to see its header (status, Start, Stop, Restart, Kill), notices, and
tabs. Tabs that do not apply to a game are hidden.

| Tab | Shown for | What it has |
| --- | --- | --- |
| Overview | All | Join addresses, Ports card, server details, start after boot, memory and CPU |
| Console | Minecraft, Hytale, Valheim | Live console with retained history. Valheim shows output only. |
| Settings | Minecraft, Valheim | Settings form |
| Files | All | Host file browser rooted at the server folder |
| Backups | All | See [Backups](https://github.com/Riqqqque/Helix/wiki/Backups) |
| Logs | All | Recent container output |
| Performance | All | CPU, memory, TPS for Minecraft |
| Marketplace | Paper-family and mod-loader Minecraft servers | Plugins or mods. Named **Mods** for Hytale and Valheim. |
| Advanced | All | Runtime identity, server software, copy to another server, memory and CPU, **Check for update**, **Remove server** |

Above the tabs the page can show:

- **Busy: …** when a job holds the server, for example `Busy: a backup. New
  actions wait in line until it finishes.`
- **N file changes waiting for a restart** when changes were staged through the
  server API. See [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).
- The modpack update banner for modpack servers.
- **Sign in to Hytale** while a Hytale server waits for a sign-in.

### Console

The console opens at the newest output and follows it until you scroll away. The
host records output even when no browser is open, across restarts, up to the
configured size limit. Minecraft commands go through RCON on `127.0.0.1` only.
That port is for the host talking to itself; it is never published to the LAN or
the internet, and Advanced shows it as **Console: Loopback only**. Helix filters
out the RCON connect and disconnect lines its own TPS sampling would add.

The TPS column for native Minecraft is a short `/tps` sample over the same RCON
channel. Paper, Purpur, Folia, Leaves, Pufferfish, and some plugins report a
number. Vanilla and most Fabric, Forge, and Quilt servers show a dash.

## Starting, stopping, and jobs

Start, Stop, Restart, Back up now, Update, and Kill run as background jobs. A job
keeps running if you close the page, and job records survive a broker restart. A
job that was running when the broker stopped is reported as failed with the stage
it had reached.

### What each action waits for

| Action | What Helix does |
| --- | --- |
| Start | Installs staged file changes, starts the container, and waits until the game is ready. Then checks that saved Minecraft settings were not overwritten. |
| Stop | Stops the server cleanly (see below). |
| Restart | Stops, installs staged changes, starts, and waits until ready. |
| Kill | Sends SIGKILL to the container right away. Unsaved progress can be lost. Asks for confirmation. Not available for imported AMP servers. |
| Back up now | See [Backups](https://github.com/Riqqqque/Helix/wiki/Backups). |
| Update / Check for update | See [Updates](#updates-and-version-changes). |

"Ready" means:

- **Minecraft:** the server answers a Minecraft status ping on its game port,
  which happens after Paper prints its `Done (…)!` line. Pumpkin must also answer
  on its Bedrock port. Helix waits up to 6 minutes, or 20 minutes for a modpack.
  If the server exits, restarts, or runs out of memory first, the job says so and
  includes the relevant startup output.
- **Every other game:** the game's runtime writes a ready marker once the server
  prints its own startup message. Helix waits up to 20 minutes.

Stopping:

- **Java Minecraft:** Helix first asks for `save-all flush` and waits up to 30
  seconds for the world save. If the console cannot confirm it, the server is not
  stopped. Then Helix stops the container with no forced-kill deadline and waits
  up to about three minutes. If the server is still shutting down, the job says so
  and nothing is killed; use **Kill** only as a last resort.
- **Pumpkin:** the same clean container stop, without the `save-all flush` step.
- **Valheim:** up to 120 seconds to save and exit.
- **Hytale:** up to 100 seconds, so its own 90-second shutdown can finish.
- **Other games:** up to 45 seconds, after which Docker stops the process.

### Busy servers and jobs waiting in line

Only one job changes a server at a time. Actions, updates, restores, installs,
deploys, and transfers wait in line behind each other instead of failing, for up
to 30 minutes. While a job waits, its stage names what it is waiting for, and the
server page shows the **Busy:** notice.

File reads never wait. File writes through the API can run side by side, but wait
a few seconds at most for a backup, restore, update, or lifecycle action; if the
server stays busy, they fail with the name of the operation that holds it and
since when.

## Settings

### Minecraft settings

Flip a switch or edit a value, then choose **Save settings** to apply it on the
next restart, or **Save & restart** to do both. The ordinary Restart button does
not apply unsaved edits. Fields that need a restart are marked, and the page keeps
a pending-restart state after you save. Controls are locked while saving, and the
editor keeps unsaved drafts during background refreshes.

Helix protects saved settings against shutdown rewrites and checks the file
values after startup. If a mod or startup script overwrites a setting, the job
names it instead of reporting success.

- **Allow flight** stops flight-related kicks. It does not let Survival players
  fly.
- Minecraft 1.21.9 and newer use the `pvp` game rule instead of the old property.
  The panel shows the console commands to read or change it.
- **Game mode** is the default for new players. Existing players and world
  difficulty can also be controlled by world data or plugins.
- The game port can be changed here. Helix rebinds the container right away,
  skips ports AMP already claims, and removes public access on the old port.

Pumpkin uses `pumpkin.toml`; see [Pumpkin](https://github.com/Riqqqque/Helix/wiki/Pumpkin).
Other games are configured through their own files in Files, or through Valheim's
Settings tab.

### Memory and CPU limits

**Allocated memory** and **CPU cap** are on Overview and Advanced. Saving either
recreates the container with the new limits and starts it again if it was
running, so players are disconnected briefly.

- Memory ranges per game are listed under [Creating a server](#creating-a-server).
  For Java Minecraft, the setting is also the Java heap (`-Xmx`); the container
  gets about 1 GiB more for Java's own overhead.
- The CPU cap is off, or 0.25 to 128 cores.

The Servers page warns when the servers' memory settings add up to more than the
machine has, and the memory setting shows the same warning before you save.
Allocating more than the machine has works only while the servers are not all
busy at once. When they are, Linux stops one of them to free memory. See
[Game Hosting and Capacity](https://github.com/Riqqqque/Helix/wiki/Game-Hosting-and-Capacity#memory).

### Start after the host boots

**Start after the host boots** is on by default when you create a server. On, the
container uses Docker's `unless-stopped` restart policy, so it comes back after
Linux or Docker restarts. Off, it stays stopped until you press Start. Changing
it does not start or stop the server now. You can change it on Overview or in
**Settings → Helix data**. This is separate from **Settings → Host integration**,
which covers the Helix dashboard and gateway containers.

### Server icons

Every native or imported server can use a Helix preset or your own PNG or JPEG.
The browser crops and resizes it and uploads at most 512 KiB. Modpack servers
default to the pack's catalog artwork; **Restore default** brings it back.

## Ports

### Join addresses

Overview shows the LAN address, a Tailscale address when one is detected, and the
public address plus game port when a WAN address is known. **Copy** copies the
address.

### The Ports card

The game port is shown with its real protocol: TCP for Minecraft Java, UDP (QUIC)
for Hytale, UDP for most Steam games. Helix manages it.

**Add port** opens another port for something inside the server that listens on
its own, such as a voice chat plugin, a web map, or Bedrock players through
Geyser. Each extra port is published on the host with the same number inside the
container, as TCP, UDP, or both. A server can have up to 16 extra ports, each 1024
or higher, with a label of up to 40 characters.

Presets match the software:

| Software | Presets |
| --- | --- |
| Paper, Purpur, Folia, Leaves, Pufferfish | Simple Voice Chat (UDP 24454), BlueMap (TCP 8100), Dynmap (TCP 8123), Geyser (UDP 19132), with their `plugins/` config paths |
| Fabric, Quilt, Forge, NeoForge | The mod versions, with their `config/` paths |
| Vanilla, Pumpkin, Hytale, other games | None; add the port the mod or tool documents |

**Save ports** recreates the container and restarts the server, so players are
disconnected for about a minute. The port must be free on the host; if another
program uses it, Helix says so. Set the same port in the plugin's own config, and
forward it on your router for players outside your network. Ports you added but
did not save stay on the card until you save or discard them.

### Port pools

**Port pools** on the Servers page sets each game's ranges and optional priority
ports. Automatic allocation tries the priority ports first, then each range in
order, skipping ports assigned to another Helix server, ports AMP claims, and
ports already in use on the host. Up to 32 ranges, 256 individual entries, and
4,096 unique ports are accepted. The summary shows capacity, assigned ports, and
the next free candidate.

### Public internet

Choosing **Public** at create time, or **Prepare host firewall** later, adds the
game's exact rules to UFW when UFW is already active. It does not turn UFW on and
never changes your router. **Remove Helix host rules** removes only the rules
that server owns. Opening a server to the internet needs the
`network.firewall.write` permission.

For players outside your network, forward the game ports in your router to the
host's LAN address:

- Minecraft Java and Terraria: the TCP game port.
- Pumpkin: TCP for Java, plus TCP and UDP for the Bedrock port.
- Hytale: the UDP game port.
- Valheim: the UDP game port and the next one.
- V Rising and Palworld: the UDP game and query ports.
- Other games: the ports listed on the Ports card.

Forward the game ports, never the Helix dashboard. Skip forwarding for LAN-only
play. Test from another network: host rules alone do not prove internet access,
and CGNAT or an upstream firewall can still block it. For Minecraft, turn on the
whitelist to control who joins.

### Server-list visibility

Some games can list your server in their in-game browser: V Rising (**Show on the
V Rising server list**), Palworld (**Show on the Palworld community list**),
Valheim (Settings), and the shared-flow games that have a list. Listing is
separate from firewall rules and router forwarding.

You can also hide the whole **Servers** page in Settings. Hiding it does not stop
any running server.

## Updates and version changes

### Minecraft: three choices

Open **Advanced → Server software**, pick one choice, tick the confirmation, and
start:

| Choice | What it does |
| --- | --- |
| **Update build** | Installs the newest build of the Minecraft version you already run. If you already have it, Helix says so and changes nothing: no backup, no restart. |
| **Change version** | Moves to a newer Minecraft version. Check that your plugins and mods support it first. Downgrades are refused because they damage worlds. |
| **Repair files** | Downloads the exact version and build you have again and rebuilds loader libraries, for a damaged or edited server JAR. |

Helix downloads and verifies the new files first, then stops the server and makes
a full backup. A running server is started on the new files and checked; if it
does not become ready, Helix restores the whole backup, including worlds and
settings. A stopped server stays stopped, so start it yourself and restore from
**Backups** if that start fails.

- Pumpkin tracks releases, so it offers **Change release** and **Repair files**.
- Modpacks keep their Minecraft and loader versions together; update them with
  the modpack banner. **Repair files** still works.
- Custom JARs have no publisher feed; replace the JAR through Files after a
  backup.

### Other games

For V Rising, Terraria, Palworld, Hytale, and the shared-flow Steam games,
**Check for update** on Advanced restarts a running server so its runtime can
fetch the current game build on start (SteamCMD for Steam games, the Hytale
Downloader for Hytale). Vanilla Terraria uses a fixed publisher download. A
stopped server is left alone and checks on its next start. Valheim updates from its Settings tab instead; see
[Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim#updates-and-repair).

### Runtime repair and upgrades

When Helix ships a newer runtime image for a game, an older server keeps its old
container until you upgrade it. Stop the server, then choose **Check for update** on
Advanced. Helix builds the new image, checks that the old container
is really this server's, and recreates it. If that fails, Helix puts the previous
container back; the server's data is not touched.

## Marketplace

The Marketplace tab installs plugins and mods that match the server's software,
loader, and Minecraft version. Toggle **Modrinth** or **CurseForge**.

- Modrinth needs no key.
- CurseForge needs an API key in **Settings → Catalogs** (from
  [console.curseforge.com](https://console.curseforge.com/)). Keep the host on a
  normal home internet connection: VPS and VPN exits are often blocked with an
  empty 403. The key is still saved when that happens.

What each server gets:

| Server | Content |
| --- | --- |
| Paper, Purpur, Leaves, Pufferfish | Plugins (CurseForge lists them as Bukkit plugins) |
| Folia | Only plugins that declare Folia support |
| Fabric, Quilt, Forge, NeoForge | Mods for that loader |
| Vanilla, custom JAR, Pumpkin | No marketplace |
| Hytale | CurseForge mods (the **Mods** tab) |
| Valheim | Thunderstore packages (the **Mods** tab) |

A missing or negative Modrinth server-side flag is shown as a warning, not a
block. Project pages render the catalog description with scripts removed and only
HTTPS images from known hosts.

**Install** on a search card, or **Review installation** on a project page, runs
as a job. Helix picks a compatible release (open the project to choose another),
adds required dependencies, verifies every file, and writes it into `plugins/` or
`mods/`. Optional dependencies are listed but never added. The server keeps
running; restart it when you want the new files loaded, or tick **Restart now and
check it starts** in the install dialog. Helix then stops the server before the
files change, starts it, waits until it is ready, and puts the previous files back
if it does not come up. This path does not take a backup, so back up first for anything risky. After a successful install the card
shows **Installed**.

Project artwork comes through Helix's own image proxy, which checks the exact CDN
path and image type and keeps recent images in memory for 30 minutes. Search
results and project details are fetched live from the catalog each time.

## Modpacks

**Start with a modpack** creates a new server from a Modrinth `.mrpack` or a
CurseForge pack. A modpack is never dropped onto an existing world.

- Modrinth packs are checked against the hashes in the pack index. Downloads have
  a 768 MiB safety cap.
- CurseForge packs need the API key. When the chosen release has an official
  server pack, Helix uses it, checks its catalog length and SHA-1, and validates
  the whole ZIP before extracting. Without one, choose a release that has server
  files; Helix does not copy a client's mod list onto a server.
- Both pin the matching loader and start an isolated server. A large first boot
  can take several minutes; Helix waits up to 20 minutes and shows the latest
  startup output if the game port never opens.

### Updating a modpack

When you open a modpack server, Helix checks its Modrinth or CurseForge project
for a newer stable release on the same Minecraft version and loader. If there is
one, the banner offers **Back up and update**. Helix never updates a pack on its
own.

Helix downloads and verifies the whole new release in a separate folder, stops
the server, and makes a full backup. It replaces only files that belong to the
pack. Worlds, player data, `server.properties`, allowlists, operator lists,
icons, logs, and locally edited configuration stay. A mod or loader JAR you
replaced yourself stops the update with an explanation instead of being
overwritten. Helix then starts the server and waits up to 20 minutes for it to
answer. If it does not, Helix restores the whole backup and the previous running
state.

Moving a pack to another Minecraft version or loader is a migration, not an
update. Create a separate server for that.

## Imported AMP servers

AMP stays separate. Helix can show an AMP instance's identity, status, players,
ports, and console data, and run the lifecycle actions the adapter supports.

- AMP Idle (sleep) is not shown as online. Helix offers **Start** to wake it.
- There is no Kill; use Stop here or kill from the AMP panel.
- Back up asks AMP to make its own backup. Restore, files, console commands,
  settings, and updates stay in AMP.
- **Open AMP** uses the configured public panel port.
- **Hide** removes an import from this browser's list without touching AMP.
  **Forget here** in **Removed and hidden** also stays in this browser.
  **Settings → Helix data** can bring it back.
- Helix never deletes an AMP instance. If AMP goes away, its imports show as
  unavailable and can be cleared.

To manage an AMP world fully in Helix, stop it and copy it into a new native
server. See [Copy a server into Helix](https://github.com/Riqqqque/Helix/wiki/Server-Migration).

## Removing and restoring servers

**Remove server** on Advanced asks you to type the server's exact name. Helix
stops and removes that server's container and moves its folder, backups, and
console history into **Removed and hidden**. Nothing is deleted yet.

From **Removed and hidden** you can:

- **Restore** the server. It comes back stopped, with its backups.
- **Delete forever** after typing the exact name again. That wipes the world,
  its Helix backups, and its console history, and cannot be undone.

Helix never empties **Removed and hidden** on its own. (The backup trash of an
active server is different; it expires after 7 days by default. See
[Backups](https://github.com/Riqqqque/Helix/wiki/Backups#automatic-expiry).)
None of this is an off-machine backup.
