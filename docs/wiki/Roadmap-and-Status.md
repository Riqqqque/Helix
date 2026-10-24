# Roadmap and Status

Helix keeps plans and evidence separate:

- [`PROGRESS.md`](https://github.com/Riqqqque/Helix/blob/main/PROGRESS.md) records
  what is implemented and how well it is verified.
- [`CHANGELOG.md`](https://github.com/Riqqqque/Helix/blob/main/CHANGELOG.md) and
  the [Releases](https://github.com/Riqqqque/Helix/releases) page list every
  change by version.
- [`NEXT.md`](https://github.com/Riqqqque/Helix/blob/main/NEXT.md) lists the next
  concrete validation work.
- [`ROADMAP.md`](https://github.com/Riqqqque/Helix/blob/main/ROADMAP.md) describes
  dependency order, not promises or dates.

## Where Helix is today

Helix is a private-LAN release: an authenticated dashboard, a typed Linux
broker, native game servers in Docker, storage and host tools, network and UFW
management, guarded package updates, Hooks, an optional non-root terminal,
installable Strands, scoped server API tokens, and a SHA-256-pinned updater.

Public-internet release remains blocked on supported-host lifecycle matrices,
an independent security review, recovery and fault drills, independently signed
artifacts, accessibility and mobile review, and broad game-version matrices.

## Release history

| Release | Highlights |
| --- | --- |
| 1.11.2 | Stopping a Minecraft server that is still starting works |
| 1.11.1 | Every Fabric loader a pack pins installs, Forge modpacks before 1.17, NeoForge 1.20.1 |
| 1.11.0 | Your own copyable address per server, colored console, typed memory sizes, containers sized for many players, Minecraft up to 64 GiB |
| 1.10.2 | Marketplace installs can restart and verify the server, the installer refuses accidental downgrades, OpenAPI covers every route, security updates |
| 1.10.1 | `job_wait` works through the gateway, the installer adds pigz, the Python client waits and deploys, the wiki is rewritten |
| 1.10.0 | Backups without stopping Minecraft, faster and smaller backups, the backup trash empties itself, file changes staged while a server runs, one-call deploy with rollback, queued jobs, memory overcommit warning |
| 1.9.0 | [One-command installer](https://github.com/Riqqqque/Helix/wiki/Getting-Started) for headless servers, tested on fresh Ubuntu 22.04 and 24.04; fresh gateway builds fixed |
| 1.8.0 | Home layout editor rebuilt: drag anywhere, resize from the corner, tiles keep their size while editing |
| 1.7.2 | Copy buttons work over plain HTTP on a LAN |
| 1.7.1 | The broker keeps running when a configured storage folder was removed, such as after uninstalling AMP |
| 1.7.0 | [Copy a setup between servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers) from the dashboard or API, with backup and automatic rollback |
| 1.6.0 | **View token** with password check and encrypted copies, **Full access** tokens, marketplace/network/removal permissions for tokens, dashboard preferences sync again |
| 1.5.2 | Ports added on the Ports card no longer disappear when the page refreshes |
| 1.5.1 | Tidier server copy dialog, rounded memory sizes, loose launcher JARs skipped |
| 1.5.0 | [Server imports](https://github.com/Riqqqque/Helix/wiki/Server-Migration) keep the Minecraft version, the port, and add-on ports; panel `server-ip` cleared |
| 1.4.0 | Ports that match each game and software, three-choice software updates, backup trash size and **Empty trash**, marketplace caching, GUI polish across every page |
| 1.3.0 | Extra container ports with presets for voice chat, web maps, and Geyser |
| 1.2.0 | [Hytale](https://github.com/Riqqqque/Helix/wiki/Hytale) servers with sign-in, CurseForge mods, and updates; a security and stability review |
| 1.1.1 | The in-app updater works on real hosts |
| 1.1.0 | Server API tokens with expiry and rotation, server file APIs and OpenAPI, safe Docker cleanup, Valheim, Pumpkin, runtime repair and version choice |
| 1.0.0 | First private-LAN release |

## Being considered next

These are ideas under consideration, not promises or dates:

- Incremental backups and quick backups of a single folder, such as one plugin.
- SHA-256 hashes in file listings and downloading a folder as one archive.
- Larger upload chunks and more than one upload per server at a time.
- A `patch_text` file action for small edits without resending a whole file.
- Helix-run maintenance jobs on the host, such as fixing permissions.
- Off-disk snapshots and server clones.

## What is intentionally not there

- Broad or unattended package upgrades and package rollback. Exact selected APT
  candidates can be applied after strict preflight and confirmation.
- Public exposure of the dashboard.
- A full client modpack copy. Modpack servers are a server-safe subset from
  Modrinth or CurseForge (CurseForge needs your own API key and a normal ISP
  exit on this host).
- Portable Wasm Strands, native sidecars, Strand signatures, or a Helix-operated
  store. Owners install UI-only Strands from a zip after reviewing the exact host
  calls they declare.
