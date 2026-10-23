# Game Hosting and Capacity

Helix creates, runs, backs up, and updates game servers, but it is not part of
the game itself. It does not run Minecraft's tick loop or carry player traffic.
How many players a server holds depends on the host's CPU, memory, disk, and
network, and on the game, its version, the world, view distance, plugins, mods,
and settings. This page helps you size a host and avoid the common ways a busy
machine runs out of room.

Helix does not publish a player-count promise. A real capacity number needs a
load test on the exact hardware, game build, world, and configuration.

## Memory

### What the memory setting means

Each server has an **Allocated memory** setting. Helix turns it into a hard
Docker memory limit with no swap:

| Server | Container limit | Inside the container |
| --- | --- | --- |
| Java Minecraft (all software, modpacks, custom JAR) | Setting + 1 GiB | Java heap `-Xmx` = setting |
| Hytale | Setting + 1 GiB | Java heap `-Xmx` = setting |
| Pumpkin | Setting | No Java heap |
| Terraria | Setting + 512 MiB | |
| V Rising, Valheim, Palworld, other Steam and dedicated games | Setting + 1 GiB | |

The extra room covers Java's own memory, Wine, and the game runtime. When a
server reaches its container limit, Linux ends that server's process. For
Minecraft, Helix reports this as `The N MiB memory allocation was exhausted.
Increase the server memory and try again`.

### Ranges and starting points

| Game | Allowed | Create default |
| --- | --- | --- |
| Minecraft Java | 1–24 GiB | 4 GiB (modpacks: suggested from the loader and free memory) |
| V Rising | 2–24 GiB | 4 GiB |
| Valheim | 1–16 GiB | 4 GiB |
| Terraria | 512 MiB – 8 GiB | 2 GiB |
| Palworld | 4–32 GiB | 8 GiB |
| Hytale | 4–32 GiB | 6 GiB |
| Satisfactory | 6–32 GiB | 12 GiB |
| Project Zomboid | 4–32 GiB | 8 GiB |
| 7 Days to Die | 6–48 GiB | 12 GiB |
| Rust | 8–64 GiB | 12 GiB |
| Sons of the Forest | 6–32 GiB | 8 GiB |
| Factorio | 1–16 GiB | 4 GiB |
| Don't Starve Together | 1–8 GiB | 2 GiB |
| Vintage Story | 2–32 GiB | 6 GiB |

These are limits and defaults, not recommendations for your world. Large Forge
and NeoForge packs often need more than 4 GiB. Start near the default, watch the
**Performance** tab under real play, and raise the setting if memory use sits
near the limit. Changing memory recreates the container and restarts a running
server.

### The overcommit warning

The Servers page warns when the servers' memory settings add up to more than the
machine has:

> Servers may use 40 GiB of memory together, but this machine has 31 GiB. If
> several get busy at once, Linux stops one of them to free memory.

The memory setting shows the same warning before you save. The total counts every
server on the list, running or stopped, so keeping some servers stopped is a
valid way to live with it.

Overcommitting works only while the servers are not all busy at once. Java
servers grow toward their heap limit over time, so "busy" can simply mean "has
been running for a while". When the machine runs out, the Linux out-of-memory
killer ends a process, usually the largest server, and its players are
disconnected.

The warning adds up the settings themselves. Each container may use up to about
1 GiB more (see the table above), and Linux, Docker, Helix, backups, and the disk
cache need memory too. Leave real headroom: as a rule of thumb, keep the total of
the settings a few GiB below the machine's memory.

## CPU

- Most game servers do most of their work on one main thread. A Minecraft
  server's tick rate depends far more on single-core speed than on core count.
  Folia is the exception among the Minecraft choices.
- The optional **CPU cap** limits a server to 0.25–128 cores. Use it to stop one
  server from starving the others, not to make a server faster.
- Backups compress on half the CPU cores, at most 8, when `pigz` is installed, so
  the other half stays free for the games. See
  [Backups](https://github.com/Riqqqque/Helix/wiki/Backups#compression-and-checks).
- First installs, SteamCMD updates, runtime image builds, and modpack downloads
  are short bursts of CPU and disk work. Run them when players are not online.
- The TPS column and the Performance tab show what a Minecraft server is doing.
  A TPS below 20 under load means the server cannot keep up.

## Disk

Plan disk space for more than the worlds:

| What | Where | Size |
| --- | --- | --- |
| Server folders | `instance_root` (installer default `/srv/helix/instances`) | The world, plugins, mods, and the game files |
| Backups | `backup_root` (installer default `/srv/helix/backups`) | Each compressed archive, times the number you keep |
| Backup trash | `backup_root/.trash` | Full size until it expires (7 days by default) |
| Files trash | `.helix-trash` in each server folder | Everything replaced or deleted through the server API; never emptied automatically |
| Staged changes | Helix's state folder | Files waiting for the next restart or deploy |
| Console history | Helix's state folder | Up to 512 MiB per server by default |
| Runtime images | Docker | One image per game and Java version |

Backups are the part that grows. Minecraft backups leave out logs, crash reports,
caches, and the Files trash, and compress well, but a long keep list of a big
world still adds up. Set a keep count, keep the trash expiry on, and empty the
Files trash once you no longer need old files.

Some operations need free space at the moment they run:

- A restore extracts the whole archive and first makes a safety backup.
- A software update, runtime repair, modpack update, Valheim mod change, and a
  deploy all make a full backup first.
- [Copying between servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers)
  needs at least twice the size of each copied part plus 1 GiB free.

Disk speed matters too. World saves, backups, and restores are heavy on writes,
and a slow or nearly full disk causes lag spikes. Fast local SSD or NVMe storage
works best. Network storage may not support the file operations Helix uses for
safe file replacement.

Backups on the same disk as the servers do not survive that disk failing. Copy
important archives to another machine.

## Network

- Players connect straight to the game port. Helix is not in the traffic path.
- Upload bandwidth is usually the limit for home hosting. Check your connection's
  upload speed against the game's guidance for the player count you expect.
- On your LAN, join with the address on the server's Overview. Tailscale gives
  friends a private address without opening ports.
- For internet play, forward only the game ports in your router, never the
  dashboard. Helix can add host firewall rules when UFW is active, but it never
  changes your router. CGNAT or an upstream firewall can still block players.
- In-game server lists (V Rising, Palworld, Valheim crossplay) and Steam or EOS
  relays can let players join without a forward. They are separate from firewall
  rules.

See [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace#ports)
for ports and [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates)
for the host side.

## How Helix stays light under load

- Game containers keep running when the dashboard or browser is closed.
- Only one job changes a server at a time. Other jobs wait in line, for up to 30
  minutes, and the server page shows what the server is busy with. Job records
  survive a broker restart.
- File reads never wait, and file writes through the API can run side by side.
- Console history uses bounded rotating files and pages of at most 500 entries.
- Host and server statistics refresh on a configurable, bounded schedule. The
  server page reuses its disk-usage measurement for a minute instead of walking
  the world on every refresh.

Helix does not grow a server's memory, change player limits under load, split a
world across machines, or create CPU or memory the host does not have.

## Helix's own footprint

The dashboard and gateway containers have Compose memory ceilings of 128 MiB and
64 MiB. Those are ceilings, not measurements, and they do not include the broker,
Docker, or any game server. Server memory is never counted as Helix memory. The
current budgets and development measurements are in
[`docs/PERFORMANCE.md`](https://github.com/Riqqqque/Helix/blob/main/docs/PERFORMANCE.md).

## Making a capacity claim

A capacity result is only meaningful with:

1. exact Helix, game, server software, Java, plugin or mod, and OS versions;
2. CPU, memory, storage, network, world, and relevant game settings;
3. the player or bot workload, its length, warmup, and the failure threshold;
4. tick health, latency, CPU, memory, disk, and network measurements; and
5. what was real, simulated, or skipped.

A synthetic test with thousands of fake players or servers can show that Helix's
own API and pages stay responsive. It cannot show that a real server holds that
many players. The full contract is in
[`docs/GAME-HOSTING-CAPACITY.md`](https://github.com/Riqqqque/Helix/blob/main/docs/GAME-HOSTING-CAPACITY.md).
