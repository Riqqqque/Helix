# Servers and Marketplace

## Choosing a game

Choose **New Server** and then a game. Minecraft is the current native Linux
runtime. V Rising is shown as a reserved, unavailable option because the current
official dedicated-server distribution is Windows-only; Helix does not disguise
an unvalidated Wine setup as one click.

The server list can be filtered by game, manager, and state. Helix-native and
imported servers keep visibly different ownership labels.

## Native Minecraft

The native creation wizard supports Paper, Purpur, Folia, Fabric, Vanilla, and
a guarded custom server JAR.
It collects the name, software, Minecraft release, memory, player limit,
automatic-pool or specific port, private/public player-access choice,
start-after-boot choice, and EULA acknowledgement. Unsupported Forge, NeoForge,
Quilt, and broad CurseForge paths remain explanations rather than fake install
buttons.

After creation, a native server has Overview, Console, Settings, Files,
Performance, Backups, Advanced, and compatible Marketplace tools. Lifecycle
buttons start, stop, restart, kill a hung container, update to the newest
compatible build for the configured Minecraft release, and back up through
bounded background jobs. Stop waits up to 45 seconds for a clean Minecraft
shutdown. Kill is a confirmed native-only SIGKILL for when that stop is stuck;
unsaved chunks can be lost. Imported AMP instances do not get Kill.
Arbitrary historical build selection is not implemented yet.

**Use your own JAR** accepts an absolute `.jar` path that already lives inside
one of Helix's configured executable-import Storage roots, an exact Minecraft
release, and Java 17, 21, or 25. The wizard includes a Storage browser as well
as direct path entry. The broker canonicalizes the path, rejects symlinks and paths
outside managed roots, bounds the file to 16 KiB–768 MiB, copies it through a
private create-new staging file, syncs and hashes it, and runs the copy as the
same isolated numeric user used by native servers. The source is untouched.
Helix cannot prove what an arbitrary JAR contains, infer its Java requirement,
offer a compatibility-filtered marketplace, or choose a publisher update, so
those actions stay manual and are labeled that way.

The root-owned broker config controls this boundary through
`native.custom_artifact_roots`. When an older config omits the field, Helix may
inherit existing narrow managed roots, but it deliberately skips `/`. If no
safe root exists, Custom JAR stays visibly unavailable instead of preventing
the broker from starting or silently trusting the whole host.

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

- Paper and Purpur receive compatible plugins;
- Folia requires content declaring Folia compatibility;
- Fabric receives matching mod JARs;
- a missing or negative Modrinth server-side flag is shown as a warning, not a
  dead end; and
- Vanilla or unsupported loaders do not get an install action.

Install re-resolves the selected project/version at the broker before download,
verifies its SHA-512 hash, and writes it only to the server's `plugins/` or
`mods/` directory. Optional dependencies are never added silently.
“Start with a modpack” is narrower: it accepts only one unambiguous stable
server-capable Fabric `.mrpack`, verifies declared hashes and extraction limits,
excludes client-only/server-optional files, and reports that the result is a
server-safe subset rather than full-pack parity.

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
address when present, and the public-internet state. **Set up public access**
uses UPnP only on the same private IPv4 gateway, refuses to overwrite an
existing router rule, requests one TCP mapping for Minecraft, verifies the
exact internal IP/port/description returned by the router, and journals
ownership before presenting a public join address. If UFW is already active,
Helix also adds one exact owned TCP rule; it never turns UFW on as a side effect.

A router-confirmed mapping is not the same as an outside test. Helix labels it
that way and recommends testing from cellular or another network. A CGNAT or
private upstream WAN address is shown as a blocker because local forwarding
cannot bypass it. Routers without compatible UPnP get a clear manual-forwarding
explanation instead of a fabricated public address.

## Port pools

Open **Port pools** on the Servers page to set Minecraft ranges and optional
individual priority ports. Automatic allocation tries the individual list
first, then each range in order, skipping duplicates, ports assigned to another
Helix server, and ports currently bound on the host. Up to 32 ranges, 256
individual entries, and 4,096 unique ports are accepted. The summary shows total
capacity, assigned ports, and the next available candidate. A ten-port range can
therefore supply ten sequential server creations without editing each wizard.

The public-setup default only preselects the visible creation choice. A public
request still requires `network.firewall.write`; failure to configure the router
does not roll back an otherwise healthy new Minecraft server, and the creation
result points back to the Join section for a safe retry.
