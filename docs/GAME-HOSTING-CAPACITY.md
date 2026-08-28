# Game Hosting Capacity Contract

## Current status

Helix currently manages Docker-backed native Minecraft instances for Paper,
Purpur, Folia, Leaves, Fabric, and Vanilla. It provides creation, lifecycle actions,
settings, persistent bounded console history, backups, and software-compatible
marketplace paths. It can also create a server-safe subset from a listed stable
server-capable Fabric `.mrpack`; Forge, NeoForge, Quilt, CurseForge, and
full-pack parity are not part of that path. AMP remains a separate optional
manager.

This is a control-plane claim, not a player-count claim. Helix is neither the
Minecraft tick loop nor a packet proxy. The host CPU, memory, storage, network,
world behavior, view/simulation distance, Java version, server software,
configuration, mods, and plugins determine real capacity.

## From one player to a busy server

The same manager handles a small and busy instance, but it does not invent
resources or automatically rewrite game settings under load.

Current bounded behavior includes:

- game containers continue running when the browser or dashboard is closed;
- each native instance has an opaque identity and explicit memory, port, player
  limit, and start-on-boot settings;
- incompatible operations on one instance are serialized or rejected;
- console capture uses bounded rotating files and cursor pages of at most 500
  entries;
- host/server metrics use a configurable bounded refresh cadence;
- creation, update, backup, and content installation expose bounded job status
  and logs; and
- software, loader, and version checks reject known-incompatible marketplace
  content instead of mixing plugins and mods, while incomplete server-side
  metadata remains a visible warning rather than a hard block; and
- Fabric `.mrpack` creation verifies declared hashes and strict archive bounds,
  excludes optional/client-only files, and atomically rolls back incomplete
  fresh instances.

Current job state is broker-lifetime state, not a durable distributed queue.
Helix does not dynamically increase a server heap, alter player caps, shard a
world, move players between hosts, or horizontally scale Minecraft.

## Resource policy

Safe capacity management means preserving operator-approved headroom:

1. Watch host CPU, memory, swap, disk, and network pressure alongside the game.
2. Give each instance a deliberate memory allocation and leave room for Linux,
   Helix, backups, Java overhead, and other workloads.
3. Treat a configured maximum-player value as game configuration, not proof the
   host can sustain that many players.
4. Schedule high-I/O updates and backups around the workload instead of assuming
   idle storage bandwidth.
5. Prefer a tested server software/configuration change over optimistic
   automatic tuning.

The current alpha does not provide a host-wide admission controller or automatic
resource rebalancer. Operators remain responsible for avoiding unsafe
overcommit.

## Interface and retention bounds

| Surface | Current boundary |
| --- | --- |
| Instance inventory | Bounded by the native/AMP manager rather than an unbounded browser fetch |
| Console history | Cursor pagination, maximum 500 entries per page, configured byte/file retention |
| Recent logs | Stable bounded tail response |
| Metrics | Configurable bounded polling; no per-player polling loop |
| Commands | One bounded typed console line with per-instance serialization |
| Jobs | Bounded progress/logs and concurrency guards; not crash-persistent |
| Backups | Exact known artifacts, protected recovery trash, bounded Undo policy |

Truncated, stale, unavailable, incompatible, or failed are explicit states.
They must not be converted into fabricated success.

## Evidence required before a capacity statement

A real capacity result must record:

1. exact Helix, Minecraft, server software, Java, mod/plugin, and OS versions;
2. CPU, memory, storage, network, world, and relevant game configuration;
3. player or bot workload, duration, warmup, sample count, and failure threshold;
4. tick/simulation health, latency, CPU, RSS, I/O, network, and Helix overhead;
5. install, start, query, console, stop, restart, update, backup, restore, and
   failure-recovery results; and
6. what was real, mocked, skipped, or unverified.

A synthetic 10,000-player or high-instance fixture can prove that Helix remains
bounded at its API/UI limits. It cannot prove a real Minecraft server supports
10,000 players. Public language must stay scoped to the exact tested hardware,
software, configuration, and workload.
