<p align="center">
  <img src="https://raw.githubusercontent.com/Riqqqque/Helix/main/docs/assets/helix-mark.png" width="96" height="96" alt="Helix logo">
</p>

# Helix Wiki

Helix is a local-first Linux dashboard for the host, its files, and game
servers. It pairs a fast web interface with an unprivileged daemon and a narrow
typed Linux broker, so useful host controls do not require a general root shell.

> Helix is a private alpha. Keep it on a network you control, do not expose it
> directly to the public internet, and do not trust it as the only copy of
> important data.

The source is licensed under `AGPL-3.0-or-later`. Open source availability does
not imply production support or a completed security review.

## Why it is useful

Helix brings the common jobs for one server into one place:

- multiple exportable Home layouts with drag-and-drop/resizable status, clock,
  weather, paged-note, and shortcut widgets plus color controls;
- live host, storage, network, service, process, and Helix resource views;
- mounted-drive browsing, bounded text editing, recoverable deletion, and
  cancellable largest-file/folder analysis;
- a native Docker-backed Minecraft manager for Paper, Purpur, Folia, Fabric,
  Vanilla, and guarded local custom-JAR imports;
- persistent bounded console history, settings with restart guidance, backups,
  a compatibility-aware Modrinth marketplace, and narrow server-safe Fabric
  `.mrpack` creation;
- optional AMP discovery and control without pretending AMP instances are
  Helix-native;
- Hooks for exact Plex, AMP, Tailscale, Pterodactyl Wings, and Jellyfin service
  discovery/control, including eligible one-click Tailscale/Jellyfin installs;
- an optional current-password-gated non-root Linux PTY; and
- selected APT updates, immediate/recurring host reboot, UFW safety activation,
  and exact Helix start-on-boot controls with explicit preflight/confirmation.

The dashboard is the control plane, not the game process or player network
path. Closing Helix does not stop a native game container. Actual player
capacity still depends on the host, world, server software, configuration, and
mods or plugins.

## Start here

Clone the source, create an owner with your own display name, and put Helix on a
private LAN by filling `.env` and the broker config with **that host's**
addresses and storage roots. Nothing ships with a demo city or demo server.

- [Getting Started](https://github.com/Riqqqque/Helix/wiki/Getting-Started)
- [How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works)
- [Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home)
- [Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files)
- [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace)
- [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates)
- [Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal)
- [Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture)
- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery)
- [Game Hosting and Capacity](https://github.com/Riqqqque/Helix/wiki/Game-Hosting-and-Capacity)
- [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands)
- [Roadmap and Status](https://github.com/Riqqqque/Helix/wiki/Roadmap-and-Status)

## Important limits

Broad/unattended package upgrades, signed Helix self-update, public-network
exposure, and a third-party Strand runtime are not implemented. Exact selected
APT candidates do have a guarded path but no rollback claim. Helix can inspect
UFW, manage exact owned allow rules, and separately enable inactive UFW only
after preserving a verified SSH listener; it cannot configure a router or prove
outside reachability. It can work behind an already configured private
Tailscale route and install/start the exact service on eligible Debian/Ubuntu
hosts, but it does not authenticate the tailnet or widen network trust.

Modpack creation is limited to listed stable server-capable Fabric releases
from Modrinth and intentionally builds a server-safe subset. Forge, NeoForge,
Quilt, CurseForge, and broad/full-pack parity are not supported.

The authoritative implementation ledger is
[`PROGRESS.md`](https://github.com/Riqqqque/Helix/blob/main/PROGRESS.md).
