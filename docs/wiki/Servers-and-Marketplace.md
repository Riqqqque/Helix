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
The host never receives Wine packages.

Create is one click after you pick a name, memory, and UDP ports. Public UPnP
is not offered. Player counts are not queried. There is no RCON command console
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
offered. There is no RCON console. For mods, drop a BepInEx pack zip at
`/data/bepinex-pack.zip` and plugin files in `/data/plugins`, then restart.
Uninstalling the last Valheim server removes that runtime image. Implemented and
unvalidated.

## Native Terraria

Helix builds `helix-terraria-runtime:1`. Vanilla downloads the publisher
dedicated zip. tModLoader uses Steam app 1281930. Edit `serverconfig.txt` in
Files. Drop `.tmod` files in `/data/mods` and restart. Public UPnP is not the
default port-pool behavior. Implemented and unvalidated.

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
retained boots. Retention is bounded, not unlimited. Settings mark fields that
need a restart and keep a pending-restart state after save.

Removing a native server stops and removes its exact container, then moves its
managed data into recoverable trash. The Removed section can restore it before
expiry. Backups have the same explicit trash-and-Undo behavior.

## Server icons

Every native or imported server can use one of the lightweight Helix presets or
a custom PNG/JPEG. The browser crops/resizes the image locally and uploads at
most 512 KiB. Stored images are validated by magic bytes and dimensions and are
served from the authenticated same-origin API.

## Modrinth

Marketplace results use real project artwork through Helix's bounded image
proxy. Image elements authenticate with the normal same-origin session cookie;
the proxy still requires `games.view`, validates the exact Modrinth CDN path,
bounds the response, and derives the media type from image bytes. Search and
details are filtered by the exact server software, loader, and Minecraft
release:

- Paper, Purpur, Leaves, and Pufferfish receive compatible plugins;
- Folia requires content declaring Folia compatibility;
- Fabric, Forge, NeoForge, and Quilt receive matching mod JARs;
- a missing or negative Modrinth server-side flag is shown as a warning, not a
  dead end; and
- Vanilla does not get an install action.

Install re-resolves the selected project/version at the broker before download,
verifies its SHA-512 hash, and writes it only to the server's `plugins/` or
`mods/` directory. Optional dependencies are never added silently.

“Start with a modpack” can search Modrinth or CurseForge without an owner API
key. Modrinth packs use `.mrpack` hash checks. CurseForge packs use the public
website catalog and `edge.forgecdn.net` files plus `manifest.json`. Both pin a
matching loader and start an isolated server. The result is a server-safe
subset, not a full client copy. Long titles and descriptions are clipped in the
browser; open the catalog or View releases for the rest.

## AMP imports

AMP stays separate. Helix can import verified instance identity, status,
players, ports, console data, and supported lifecycle actions that the adapter
can prove. Helix does not force-kill AMP instances; use Stop here or kill from
the AMP panel. **Open AMP** uses the configured public panel port, not the private
loopback API port. AMP-only settings stay in AMP.

Hide removes an import from this browser's list without touching AMP. Deleting
the upstream AMP instance must happen in AMP; Helix will not reinterpret a Hide
button as destructive upstream deletion. If AMP disappears, inventory becomes
unavailable/degraded and stale imports can be cleared instead of crashing the
dashboard.

## Join addresses

Each native detail view shows a LAN address, a separately detected Tailscale
address when present, and the public-internet state. Minecraft **Set up public
access** uses UPnP only on the same private IPv4 gateway, refuses to overwrite
an existing router rule, requests one TCP mapping, verifies the exact internal
IP/port/description returned by the router, and journals ownership before
presenting a public join address. If UFW is already active, Helix also adds one
exact owned TCP rule; it never turns UFW on as a side effect. V Rising stays
private; Helix does not offer UPnP for its UDP game and query ports.

A router-confirmed mapping is not the same as an outside test. Helix labels it
that way and recommends testing from cellular or another network. A CGNAT or
private upstream WAN address is shown as a blocker because local forwarding
cannot bypass it. Routers without compatible UPnP get a clear manual-forwarding
explanation instead of a fabricated public address.

## Port pools

Open **Port pools** on the Servers page to set Minecraft or V Rising ranges and
optional individual priority ports. Automatic allocation tries the individual
list first, then each range in order, skipping duplicates, ports assigned to
another Helix server, and ports currently bound on the host. Up to 32 ranges,
256 individual entries, and 4,096 unique ports are accepted. The summary shows
total capacity, assigned ports, and the next available candidate.

The Minecraft public-setup default only preselects the visible creation choice.
A public request still requires `network.firewall.write`; failure to configure
the router does not roll back an otherwise healthy new Minecraft server, and the
creation result points back to the Join section for a safe retry. V Rising
cannot enable that default.
