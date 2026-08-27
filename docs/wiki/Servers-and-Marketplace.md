# Servers and Marketplace

## Choosing a game

Choose **New Server** and then a game. Minecraft is the current native Linux
runtime. V Rising is shown as a reserved, unavailable option because the current
official dedicated-server distribution is Windows-only; Helix does not disguise
an unvalidated Wine setup as one click.

The server list can be filtered by game, manager, and state. Helix-native and
imported servers keep visibly different ownership labels.

## Native Minecraft

The native creation wizard supports Paper, Purpur, Folia, Fabric, and Vanilla.
It collects the name, software, Minecraft release, memory, player limit, port,
start-after-boot choice, and EULA acknowledgement. Unsupported Forge, NeoForge,
Quilt, and broad CurseForge paths remain explanations rather than fake install
buttons.

After creation, a native server has Overview, Console, Settings, Files,
Performance, Backups, Advanced, and compatible Marketplace tools. Lifecycle
buttons start, stop, restart, update to the newest compatible build for the
configured Minecraft release, and back up through bounded background jobs.
Arbitrary historical build selection is not implemented yet.

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
proxy. Search and details are filtered by the exact server software and
Minecraft release:

- Paper and Purpur receive compatible plugins;
- Folia requires content declaring Folia compatibility;
- Fabric receives compatible server-side mods; and
- Vanilla or unsupported loaders do not get an install action.

Install re-resolves the selected project/version at the broker before download.
“Start with a modpack” is narrower: it accepts only one unambiguous stable
server-capable Fabric `.mrpack`, verifies declared hashes and extraction limits,
excludes client-only/server-optional files, and reports that the result is a
server-safe subset rather than full-pack parity.

## AMP imports

AMP stays separate. Helix can import verified instance identity, status,
players, ports, console data, and supported lifecycle actions that the adapter
can prove. **Open AMP** uses the configured public panel port, not the private
loopback API port. AMP-only settings stay in AMP.

Hide removes an import from this browser's list without touching AMP. Deleting
the upstream AMP instance must happen in AMP; Helix will not reinterpret a Hide
button as destructive upstream deletion. If AMP disappears, inventory becomes
unavailable/degraded and stale imports can be cleared instead of crashing the
dashboard.

## Join addresses

Each detail view separates local address, private/Tailscale candidates, port
publication, listener evidence, firewall evidence, and outside reachability.
Helix can diagnose a missing listener or UFW allowance and offer only the safe
fix it can verify. Router forwarding, CGNAT, DNS, and true internet reachability
remain external and are never guessed.
