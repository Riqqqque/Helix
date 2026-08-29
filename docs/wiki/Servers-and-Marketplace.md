# Servers and Marketplace

## Choosing a game

Choose **New Server** and then a game. Minecraft is the current native Linux
Java runtime. V Rising, Valheim, and Terraria install dedicated servers into
isolated Helix containers. The chooser uses original Helix marks, not
publisher artwork.

The server list can be filtered to **Helix** (native servers only), Minecraft,
V Rising, Valheim, Terraria, or imported Connections. Helix-native and imported
servers keep visibly different ownership labels.

Owner setup asks whether to include the Servers page. Existing owners keep it.
Settings can hide or restore it later; hiding the page does not stop running
game containers.

## Native V Rising

V Rising is not a Linux dedicated server. Helix builds `helix-vrising-runtime:1`
from an embedded Dockerfile the first time you create a V Rising server. Game
files, saves, and the isolated runtime live in that instance’s data directory.
The host never receives Wine packages. That first image build writes Docker CLI
state under Helix native data, not the execution backend’s home directory, so
it works with helix-privd’s locked-down service.

Create is one click after you pick a name, memory, and UDP ports. Public UPnP
is not offered. Allocated memory can be changed later from Overview. Player
counts are not queried. There is no RCON command console
and no Modrinth marketplace. Backups, start-on-boot, files, and logs work the
same way as Minecraft. Host settings live in `save/Settings/ServerHostSettings.json`
under Files. Update restarts the container so SteamCMD runs again.
Uninstalling the last V Rising server removes the runtime image.

When the last **active** V Rising server is removed, Helix deletes the runtime
image. Trashed data stays recoverable; restore rebuilds the image if needed.
This path is implemented and unvalidated. It is not publisher-supported.

## Native Valheim

Helix builds `helix-valheim-runtime:1` and installs Steam dedicated app 896660.
Create is private-LAN UDP: the game port plus the next two. Public UPnP is not
offered. Allocated memory can be changed later from Overview. There is no RCON console. For mods, drop a BepInEx pack zip at
`/data/bepinex-pack.zip` and plugin files in `/data/plugins`, then restart.
Uninstalling the last Valheim server removes that runtime image. Implemented and
unvalidated.

## Native Terraria

Helix builds `helix-terraria-runtime:1`. Vanilla downloads the publisher
dedicated zip. tModLoader uses Steam app 1281930. Edit `serverconfig.txt` in
Files. Drop `.tmod` files in `/data/mods` and restart. Allocated memory can be
changed later from Overview. Public UPnP is not the default port-pool behavior.
Implemented and unvalidated.

## Start on boot

Native Helix servers persist a start-on-boot flag in the instance manifest and
set Docker `--restart unless-stopped` or `no`. Creation still starts the first
time so the runtime can install. The later Overview toggle changes policy only;
it does not start or stop the server now. After a host reboot, Docker brings
back servers that opted in. This is separate from the Settings control that
only covers the Helix dashboard and gateway containers.

## Native Minecraft

The native creation wizard supports Paper, Purpur, Folia, Leaves, Fabric, Forge,
NeoForge, Quilt, Pufferfish, Vanilla, and a guarded custom server JAR. The Minecraft version field loads published
releases for the selected software, including an explicit Latest stable choice
except for custom JARs. Paper, Folia, and Leaves omit experimental Minecraft
versions that create would refuse. The wizard collects the name, software, Minecraft release, memory, player limit,
automatic-pool or specific port, private/public player-access choice,
start-after-boot choice, and EULA acknowledgement. Paper, Purpur, Folia,
Leaves, Fabric, Forge, NeoForge, Quilt, Pufferfish, and Vanilla are default
create choices. Forge uses the official installer for Minecraft 1.17 and newer
and launches with generated `unix_args`. Pufferfish comes from the publisher
CI over HTTPS without a checksum pin, same honesty as Purpur.

After creation, a native server has Overview, Console, Settings, Files,
Performance, Backups, Advanced, and compatible Marketplace tools. Lifecycle
buttons start, stop, restart, kill a hung container, update to the newest
compatible build for the configured Minecraft release, and back up through
bounded background jobs. Stop waits up to 45 seconds for a clean Minecraft
shutdown. Kill is a confirmed native-only SIGKILL for when that stop is stuck;
unsaved chunks can be lost. Imported AMP instances do not get Kill.
Arbitrary historical Paper/Purpur build IDs are not selectable; Helix still
updates to the newest compatible build for the chosen Minecraft release.

**Use your own JAR** accepts a dropped `.jar` from this computer, an Upload
picker, or an absolute path that already lives inside Storage. Dropped folders
are rejected. Uploads go through bounded JSON chunks into Helix's private
import root (16 KiB–768 MiB, ZIP magic on the first chunk). Storage-browser
paths still have to sit inside a configured executable-import root. The wizard
also asks for the exact Minecraft release and Java 17, 21, or 25.
Helix cannot prove what an arbitrary JAR contains, infer its Java requirement,
offer a compatibility-filtered marketplace, or choose a publisher update, so
those actions stay manual and are labeled that way.

Helix always keeps a private `{state_root}/imports` directory for dropped JARs.
Extra Storage paths can be added with `native.custom_artifact_roots`; `/` is
never inherited as an import root. If the native manager is down, Custom JAR
stays visibly unavailable instead of silently trusting the whole host.

The console opens at the newest output and follows it until the operator scrolls
away. History is captured by the host even when no browser is open and spans
retained boots. Retention is bounded, not unlimited. Minecraft commands use
RCON on 127.0.0.1 only. That is loopback: the host talking to itself. Players
never use that port, and it is not published to the LAN or internet. The
Advanced tab labels this **Console: Loopback only** and the info tip explains
the same thing. Helix also drops the noisy “Thread RCON Client … started /
shutting down” lines that TPS sampling used to spam into the log.

The list TPS column for native Minecraft is a short local `/tps` sample over
that same RCON channel, not the status ping. Paper, Purpur, Folia, Leaves,
Pufferfish, and some plugins report a number. Vanilla and most Fabric/Forge/Quilt
servers stay as an em dash. Imported AMP servers still use AMP’s TPS metric.
Settings mark fields that need a restart and keep a pending-restart state after
save. The game port can be changed there; Helix rebinds the published container
immediately, skips ports AMP already has claimed, and removes public access on
the old port. Allocated memory can be changed from Settings or Overview; Helix
rebinds the container so Docker and Minecraft `-Xmx` both pick up the new limit.

Removing a native server stops and removes its exact container, then moves its
managed data into recoverable trash. The Removed section can restore it before
expiry.

## Backups

**Back up now** stops a running server, archives the data folder, then starts it
again. Restore replaces the live data with that archive.

You can delete any backup at any time. The default delete moves it to
**Deleted backups**, where **Undo** puts it back. **Delete forever** (from the
active list or from trash, after a confirm) removes it from disk.

Each server can keep a maximum number of backups (1–50, or 0 for no count
limit) and/or a maximum age in days (1–365, or 0 for no age limit). After a
backup, or when you save / apply those rules, extras move to trash oldest first.
The newest copy from the backup that just finished is kept even if the count
would otherwise drop it. Trash is not auto-purged; delete forever is always a
person clicking it.

## Server icons

Every native or imported server can use one of the lightweight Helix presets or
a custom PNG/JPEG. The browser crops/resizes the image locally and uploads at
most 512 KiB. Stored images are validated by magic bytes and dimensions and are
served from the authenticated same-origin API.

## Marketplace

Marketplace results use real project artwork through Helix's bounded image
proxy. Image elements authenticate with the normal same-origin session cookie.
Modrinth icons go through `/api/v1/marketplace/modrinth/image`; CurseForge
avatars go through `/api/v1/marketplace/curseforge/image`. Both still require
`games.view`, validate the exact CDN path, bound the response, and derive the
media type from image bytes.

Toggle **Modrinth** or **CurseForge**. Paper-family servers get plugins (CurseForge
lists those as Bukkit plugins / addons). Fabric, Forge, NeoForge, and Quilt get
mods, and on CurseForge they can also browse **Modpacks**. A modpack JAR is not
dropped onto an existing world; create a new server from **Start with a modpack**
instead. Search and details stay filtered by the exact server software, loader,
and Minecraft release:

- Paper, Purpur, Leaves, and Pufferfish receive compatible plugins;
- Folia requires content declaring Folia compatibility;
- Fabric, Forge, NeoForge, and Quilt receive matching mod JARs;
- a missing or negative Modrinth server-side flag is shown as a warning, not a
  dead end; and
- Vanilla does not get an install action.

Project pages render the catalog description the way Modrinth and CurseForge
write it (markdown or HTML), with scripts stripped and only https images from
known hosts. After a successful install the search card and project header show
**Installed**.

**Install** on a search card, or **Review installation** on the project page,
writes the verified JAR into that server's `plugins/` or `mods/` folder and
leaves Minecraft running. Helix picks a compatible release automatically on the
list button; open the project if you want a different build. Restart the server
yourself when you want the files loaded. Helix does not take a world backup for
this path. Optional dependencies are never added silently.

“Start with a modpack” can search Modrinth or CurseForge without an owner API
key. Modrinth packs use `.mrpack` hash checks. CurseForge packs use the public
website catalog and `edge.forgecdn.net` files plus `manifest.json`. Both pin a
matching loader and start an isolated server. The result is a server-safe
subset, not a full client copy. Long titles and descriptions are clipped in the
browser; open the catalog or View releases for the rest.

## AMP imports

AMP stays separate. Helix can import verified instance identity, status,
players, ports, console data, and supported lifecycle actions that the adapter
can prove. AMP Idle (sleep) is not treated as online: the game is hibernated,
the AMP manager is often still running, and Helix offers Start to wake it
instead of Restart. Stopped, starting, failed, and AMP-manager-stopped states
are shown as themselves. Helix does not force-kill AMP instances; use Stop here
or kill from the AMP panel. **Open AMP** uses the configured public panel port,
not the private loopback API port. AMP-only settings stay in AMP.

Hide removes an import from this browser's list without touching AMP. Deleting
the upstream AMP instance must happen in AMP; Helix will not reinterpret a Hide
button as destructive upstream deletion. If AMP disappears, inventory becomes
unavailable/degraded and stale imports can be cleared instead of crashing the
dashboard.

## Join addresses

Each native detail Overview shows a LAN address, a separately detected Tailscale
address when present, and the public IP plus the game port when Helix can see a
WAN address. That public row is the address people would use from the internet.
Helix does not set up router forwarding from Overview. If players should join
from outside the LAN, port-forward the game port on the router: TCP for
Minecraft and Terraria, UDP game plus query for V Rising, and UDP
`game` through `game+2` for Valheim.

Minecraft create can still request UPnP public access on the same private IPv4
gateway and refuses to overwrite an existing router rule.

If Helix says AMP already has a port claimed, it is refusing to steal a number
AMP still lists. Open AMP (the error card has a link when Helix can see the
instance), stop that instance, open **Configuration → Server Settings / Portals**,
change the listed port to a free number, Apply, then retry. You can also leave
AMP on that number and let Helix auto-pick from **Port pools**; automatic create
already skips AMP numbers. Helix will not edit AMP instance files, kvp, or
`server.properties`.

If the AMP instance is already gone and only a leftover UPnP mapping remains
(description starts with `AMP`, no instance file still lists the port), Helix
asks you to type `REMOVE AMP FORWARD <port>`. That deletes the leftover router
forward only. It does not stop AMP or rewrite AMP files. Helix-owned public
access on that port is removed from the Helix server instead.

Change the Helix game port in Settings if you want Helix to use a different
number. Public setup requests one TCP mapping, verifies the exact internal
IP/port/description returned by the router, and journals ownership. If UFW is
already active, Helix also adds one exact owned TCP rule; it never turns UFW on
as a side effect. V Rising stays private at create; Helix does not offer UPnP
for its UDP game and query ports.

A router-confirmed mapping is not the same as an outside test. Helix labels it
that way and recommends testing from cellular or another network. A CGNAT or
private upstream WAN address is shown as a blocker because local forwarding
cannot bypass it. Routers without compatible UPnP get a clear manual-forwarding
explanation instead of a fabricated public address.

## Port pools

Open **Port pools** on the Servers page to set Minecraft or V Rising ranges and
optional individual priority ports. Automatic allocation tries the individual
list first, then each range in order, skipping duplicates, ports assigned to
another Helix server, ports AMP already has claimed, and ports currently bound
on the host. AMP-claimed numbers in the pool are listed on that dialog. Up to 32 ranges,
256 individual entries, and 4,096 unique ports are accepted. The summary shows
total capacity, assigned ports, and the next available candidate.

The Minecraft public-setup default only preselects the visible creation choice.
A public request still requires `network.firewall.write`; failure to configure
the router does not roll back an otherwise healthy new Minecraft server, and the
creation result points back to the Join section for a safe retry. V Rising
cannot enable that default.
