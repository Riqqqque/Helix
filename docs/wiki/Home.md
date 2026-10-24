<p align="center">
  <img src="https://raw.githubusercontent.com/Riqqqque/Helix/main/docs/assets/helix-mark.png" width="96" height="96" alt="Helix logo">
</p>

<h1 align="center">Helix Wiki</h1>

<p align="center">
  <strong>One private dashboard for a Linux host, its storage, and your game servers.</strong><br>
  Version 1.11.2 · <a href="https://github.com/Riqqqque/Helix/releases/latest">Latest release</a> ·
  <a href="https://github.com/Riqqqque/Helix">Repository</a>
</p>

Helix runs your game servers in isolated containers, backs them up without
taking them offline, lets scripts deploy plugins safely, and keeps the everyday
controls for one Linux machine in a single fast web dashboard. A Preact
interface talks to an unprivileged Rust service and a narrow, typed Linux broker,
so useful host controls never turn the browser into a root shell.

> [!CAUTION]
> Helix is a private-LAN release. Keep it on a network you control, do not expose
> the dashboard directly to the public internet, and do not treat it as the only
> copy of important data.

## Start here

Helix is made for a headless Linux server. SSH in and run:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash
```

It installs everything, then prints the address to open from your laptop and a
one-time owner token.

| If you want to… | Read |
| --- | --- |
| Install Helix and log in for the first time | [Getting Started](https://github.com/Riqqqque/Helix/wiki/Getting-Started) |
| See every installer option and configuration key | [Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration) |
| Create and run a game server | [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace) |
| Back up without stopping players | [Backups](https://github.com/Riqqqque/Helix/wiki/Backups) |
| Upload plugins and deploy them safely | [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes) |
| Script Helix from a build pipeline | [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API) |
| Fix something that is not working | [Troubleshooting and FAQ](https://github.com/Riqqqque/Helix/wiki/Troubleshooting-and-FAQ) |

## What's new in 1.10.0

- **Live backups.** A running Minecraft server is archived while players stay
  connected; autosave pauses for the archive and always turns back on.
- **Faster, smaller backups.** Compression uses several cores, and logs, crash
  reports, caches, and the Files trash are left out.
- **The backup trash empties itself** after the number of days you choose
  (7 by default).
- **Staged changes and one-call deploys.** Files written through the API while a
  server runs wait for the next restart, or a Deploy job backs up, installs,
  restarts, waits until the server is ready, and rolls back if it does not come
  up.
- **Jobs wait in line** and say what they are waiting for; `job_wait` replaces
  polling; job records survive a broker restart.
- **A memory warning** when servers together may use more memory than the
  machine has.

The full list is in the
[changelog](https://github.com/Riqqqque/Helix/blob/main/CHANGELOG.md).

## Using Helix

- [Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home) — pages, Home layouts, widgets,
  themes, and settings.
- [Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files) — browse, edit, and find what fills a
  drive.
- [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates) — addresses, firewall,
  packages, reboots, Docker cleanup, and updating Helix.
- [Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal) — Plex, Tailscale, Jellyfin, Wings,
  Docker, and the optional terminal.

## Game servers

- [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace) — every supported game,
  console, files, settings, ports, updates, plugins, mods, and modpacks.
- [Backups](https://github.com/Riqqqque/Helix/wiki/Backups) — live backups, keep rules, the trash, and restores.
- [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes) — staged changes and the
  Deploy job.
- [Hytale](https://github.com/Riqqqque/Helix/wiki/Hytale), [Pumpkin](https://github.com/Riqqqque/Helix/wiki/Pumpkin),
  and [Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim) — game-specific guides.
- [Copy a server into Helix](https://github.com/Riqqqque/Helix/wiki/Server-Migration) — bring an AMP or Pterodactyl
  server over with the same version and port.
- [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers) — promote a test setup to
  your public server.
- [Game Hosting and Capacity](https://github.com/Riqqqque/Helix/wiki/Game-Hosting-and-Capacity) — sizing memory, CPU,
  and disk.

## API and automation

- [API and Integrations](https://github.com/Riqqqque/Helix/wiki/API-and-Integrations) — how the API is organised,
  authentication, errors, jobs, and the OpenAPI contract.
- [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API) — every token operation, with curl,
  Python, and PowerShell examples.
- [Dashboard API Reference](https://github.com/Riqqqque/Helix/wiki/Dashboard-API-Reference) — every owner endpoint.
- [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens) — create, scope, view, rotate, and
  revoke tokens.
- [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands) — shareable dashboard pages and widgets.

## Operations and internals

- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery) — boundaries, Helix's own
  backups, and what to do when something breaks.
- [Troubleshooting and FAQ](https://github.com/Riqqqque/Helix/wiki/Troubleshooting-and-FAQ) — common problems and
  where to look.
- [How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works) — the request path from browser to host.
- [Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture) — crates, data stores, and privilege
  boundaries.
- [Development and Testing](https://github.com/Riqqqque/Helix/wiki/Development-and-Testing) — build, test, and
  release.
- [Glossary](https://github.com/Riqqqque/Helix/wiki/Glossary) — Helix terms in one place.
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
- Give scripts scoped API tokens that can deploy, back up, and manage exactly
  the servers you choose, and nothing on the host.
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
Questions go to [Discussions](https://github.com/Riqqqque/Helix/discussions),
bugs to [Issues](https://github.com/Riqqqque/Helix/issues), and security problems
to a [private report](https://github.com/Riqqqque/Helix/security/advisories/new).

The source is licensed under `AGPL-3.0-or-later`. Open source availability does
not imply production support or a completed security review.
