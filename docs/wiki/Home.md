<p align="center">
  <img src="https://raw.githubusercontent.com/Riqqqque/Helix/main/docs/assets/helix-mark.png" width="96" height="96" alt="Helix logo">
</p>

# Helix Wiki

Helix is one private dashboard for a Linux host, its storage, and your game
servers. A fast web interface talks to an unprivileged daemon and a narrow, typed
Linux broker, so useful host controls never require a general root shell.

> [!CAUTION]
> Helix is a private-LAN release. Keep it on a network you control, do not expose
> the dashboard directly to the public internet, and do not treat it as the only
> copy of important data.

## Start here

New to Helix? It is made for a headless Linux server. SSH in and run:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash
```

It installs everything, then prints the address to open from your laptop and a
one-time owner token.

1. [Getting Started](https://github.com/Riqqqque/Helix/wiki/Getting-Started) — requirements, the installer, and its options.
2. [Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home) — layouts, widgets, themes.
3. [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace) — create and run game
   servers, plugins, mods, modpacks, ports, and backups.

## Game servers

- [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace) — Minecraft and every other
  game: console, files, settings, ports, updates, backups, marketplace.
- [Hytale](https://github.com/Riqqqque/Helix/wiki/Hytale) — sign-in, UDP ports, CurseForge mods, updates.
- [Pumpkin](https://github.com/Riqqqque/Helix/wiki/Pumpkin) — the native Rust Minecraft server.
- [Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim) — world rules, crossplay, Thunderstore mods.
- [Copy a server into Helix](https://github.com/Riqqqque/Helix/wiki/Server-Migration) — bring an AMP or Pterodactyl
  server over with the same version and port.
- [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers) — promote a test setup to
  your public server.
- [Game Hosting and Capacity](https://github.com/Riqqqque/Helix/wiki/Game-Hosting-and-Capacity) — what to expect from
  your hardware.

## Automation and AI

- [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens) — Full access in one click, view a token
  again, and hand a token to an AI assistant safely.
- [API and Integrations](https://github.com/Riqqqque/Helix/wiki/API-and-Integrations) — the HTTP API, OpenAPI contract,
  and Python client.
- [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands) — shareable dashboard pages and widgets.

## Host and operations

- [Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files) — browse, edit, and find what fills a
  drive.
- [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates) — addresses, firewall,
  packages, reboots, and updating Helix.
- [Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal) — Plex, Tailscale, Jellyfin, Wings,
  Docker, and the optional terminal.
- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery) — boundaries, backups, and
  what to do when something breaks.

## Under the hood

- [How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works) — the request path from browser to host.
- [Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture) — the crates and their responsibilities.
- [Development and Testing](https://github.com/Riqqqque/Helix/wiki/Development-and-Testing) — build and test locally.
- [Roadmap and Status](https://github.com/Riqqqque/Helix/wiki/Roadmap-and-Status) — releases so far and what comes
  next.

## What Helix can do

- Run Minecraft (Paper, Purpur, Folia, Leaves, Pufferfish, Fabric, Quilt, Forge,
  NeoForge, Vanilla, Pumpkin, or your own JAR), Hytale, Valheim, V Rising,
  Terraria and tModLoader, Palworld, Satisfactory, Project Zomboid, 7 Days to
  Die, Rust, Sons of the Forest, Factorio, Don't Starve Together, and Vintage
  Story in isolated containers.
- Install plugins, mods, and modpacks from Modrinth and CurseForge, filtered to
  the server's software and version.
- Update, repair, back up, and restore servers with automatic rollback.
- Open extra ports with presets that match each game and server software.
- Copy a finished setup from one server to another, and bring servers in from
  AMP or Pterodactyl without changing their address.
- Hand scoped, viewable API tokens to scripts and AI assistants.
- Watch the host, clean Docker safely, manage the firewall, apply selected
  package updates, schedule reboots, and update Helix itself from a pinned
  release.
- Arrange a Home page of widgets, notes, and shortcuts that follows you between
  browsers.

The dashboard manages servers; it is not the game process or the player's
network path. Closing Helix does not stop a server, and player capacity still
depends on the host, the world, the server software, and its plugins or mods.

## Honest limits

Helix does not expose the dashboard publicly, configure routers without
compatible UPnP, prove a game port is reachable from outside, perform broad
unattended upgrades, or replace off-host backups. Exact selected APT candidates
have a guarded path with no rollback promise. Tailscale is installed and started
on eligible hosts, but you sign in yourself.

What is verified and what is still open is tracked in
[`PROGRESS.md`](https://github.com/Riqqqque/Helix/blob/main/PROGRESS.md). Release
notes are on the [Releases](https://github.com/Riqqqque/Helix/releases) page.

The source is licensed under `AGPL-3.0-or-later`. Open source availability does
not imply production support or a completed security review.
