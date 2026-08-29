# Game Hosting and Capacity

Helix has a native Docker-backed manager for Minecraft (Paper, Purpur, Folia,
Leaves, Fabric, Forge, NeoForge, Quilt, Pufferfish, Vanilla), V Rising, Valheim,
and Terraria. It can create and operate instances, retain bounded console
history, manage settings and backups, and install compatible Modrinth content
for the supported plugin/mod profiles. “Start with a modpack” additionally
creates a server-safe subset from Modrinth `.mrpack` or public CurseForge
`manifest.json` packs; it does not promise client/full-pack parity. AMP remains a
separate optional manager.

That does not produce a universal player limit. Helix sits outside Minecraft's
tick loop and player network path. Actual capacity depends on CPU single-thread
performance, memory, storage latency, network, world behavior, view distance,
server software, Java, mods/plugins, and configuration.

## What scales today

- Native game containers keep running when the dashboard is closed.
- Per-instance locks reject incompatible concurrent actions.
- Console history uses bounded rotating files and cursor pages of at most 500
  entries.
- Host and game statistics use a configurable bounded refresh cadence.
- Background creation, update, backup, and content jobs expose bounded status
  and logs.
- Server memory, CPU cap, port, player limit, and start-on-boot choices are explicit.

Current jobs are broker-lifetime state rather than a durable crash-persistent
queue. Helix also does not auto-scale a Minecraft process, rewrite its memory or
player limit under load, shard worlds, or create CPU/RAM that the host lacks.

A synthetic large-instance/player fixture can prove the API and UI remain
bounded. It cannot prove a real server supports the same player count. A public
capacity statement needs the exact game build, hardware, Java, world,
configuration, plugins/mods, sample count, and failure threshold.

The full contract is in
[`docs/GAME-HOSTING-CAPACITY.md`](https://github.com/Riqqqque/Helix/blob/main/docs/GAME-HOSTING-CAPACITY.md).
