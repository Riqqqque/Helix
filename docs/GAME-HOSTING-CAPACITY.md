# Game Hosting Capacity Contract

## Current status

Game-server management is not implemented. This document is the performance and
safety contract for that future work; it is not evidence that Helix can install,
run, tune, or scale a game today.

Helix will be a control plane, not the game server and not a packet proxy. The
game process owns simulation ticks, networking, world data, plugins or mods, and
the actual player limit. Helix can keep its own overhead small, allocate bounded
host resources, and show useful pressure signals. It cannot make an overloaded
game engine or undersized host support unlimited players.

## One player and many players

The same architecture must cover both ends without separate “small server” and
“large server” modes:

- with no managed instances, game hosting adds no timer, poll, connection,
  worker, cache, or database writer;
- each game runs as an independent systemd unit inside its own cgroup, so a
  dashboard restart does not restart the game;
- summary work scales with visible instances, not with every connected player;
- player, log, console, and file detail is fetched only while its view is open;
- high-cardinality results use bounded cursor pages and never one unbounded
  response, database transaction, channel, or DOM tree;
- one shared reconnectable event stream carries coalesced changes instead of a
  polling loop per card, instance, or player;
- bounded queues apply backpressure, and a slow dashboard client is disconnected
  and resumes from a cursor instead of accumulating memory;
- expensive install, update, backup, restore, and archive work runs as explicit
  jobs with global and per-instance concurrency limits.

The UI must summarize first, load detail on demand, and window long lists. A
10,000-player control-plane fixture may prove bounded API and rendering behavior;
it does not prove that a real game can host 10,000 players.

## Resource policy

“Dynamic” must mean adapting within an operator-approved envelope, not silently
rewriting critical game settings.

1. Measure host CPU, memory, storage, and I/O pressure and reserve configured
   headroom for Linux, Helix, backups, and other workloads.
2. Give every instance explicit minimum, target, and hard resource bounds where
   the game and systemd support them.
3. Reject a start or high-cost job when its declared reservation cannot fit
   safely. Show the exact limiting resource.
4. Prefer CPU weight, memory ceilings, I/O priority, job scheduling, and clear
   recommendations over changing a game's player cap or runtime heap live.
5. Never overcommit memory by default. Any opt-in overcommit policy needs visible
   risk, a host-wide ceiling, and an automatic stop on new admissions before the
   operating system is forced into an unsafe state.
6. Treat game-specific tuning as versioned integration policy backed by current
   upstream documentation and real lifecycle/load evidence.

Single-host scheduling cannot create CPU, RAM, disk throughput, or network
bandwidth. Horizontal scaling, proxies, sharding, and multi-node orchestration
are separate future features and must not be implied by the local-first design.

## Bounded interface rules

Exact defaults belong in the implementation ADR, but these limits are mandatory:

| Surface | Required behavior |
| --- | --- |
| Instance lists | Cursor pagination with a server-enforced page maximum |
| Player lists | Bounded pages and windowed rendering; no fetch-all endpoint |
| Console and logs | Bounded tail, bounded line/event sizes, resumable cursor, explicit truncation |
| Live metrics | Coalesced summaries, adaptive cadence, no persistence unless enabled |
| Commands | Bounded input/output, authorization, timeout, cancellation, and per-instance serialization where required |
| Jobs | Durable intent, idempotency, host-wide fairness, incompatible-operation locks, bounded progress history |
| Database state | Configuration and durable intent only; no unbounded per-player telemetry in critical state |

Limits must be visible in API schemas and tested at the boundary. Truncation,
omission, overload, and stale data are explicit states, never silently fabricated
success.

## Evidence required before a capacity claim

Game hosting stays **NOT STARTED** until implementation begins, and it cannot be
called scalable until all of the following pass:

1. A non-game fixture proves independent systemd/cgroup lifetime, resource
   limits, start admission, cancellation, crash-loop handling, and daemon
   restart behavior.
2. Disabled hosting has no measurable idle task or write cost.
3. Bounded synthetic instance/player/log streams prove memory, API latency,
   reconnect, overload, and UI long-task behavior at and beyond every limit.
4. A real current game build passes install, configure, start, query, player
   visibility, graceful stop, restart, update, backup, restore, and failure
   recovery on each claimed Ubuntu/version combination.
5. Small, representative, and saturation load runs record player count,
   tick/simulation health, CPU, RSS, storage and network pressure, game
   configuration, plugins/mods, hardware, and Helix's incremental overhead.
6. Published capacity language names the exact game, version, hardware,
   configuration, sample count, and failure threshold. It never turns a lab
   maximum into a universal player guarantee.

The game integration owns the real player-capacity evidence. Helix owns proof
that observing and managing it remains bounded, responsive, recoverable, and out
of the game's hot path.
