# Performance Budget and Evidence

## Status

Helix does not yet have a reference Ubuntu baseline. The numbers below are
development snapshots and engineering limits, not a RAM promise or player-count
guarantee.

The Overview/Host surfaces report current Helix-only resource use separately
from managed game servers. That live value is more useful for one installation
than an old development measurement, but it still needs workload and time
context.

## Enforced and configured limits

| Measurement | Current limit | Meaning |
| --- | ---: | --- |
| Initial frontend transfer | 75 KiB gzip | Build fails when the first usable route exceeds the budget |
| Initial JavaScript | 40 KiB gzip | Lazy feature chunks are counted separately |
| Dashboard container memory | 128 MiB | Compose ceiling, not a reservation or measured idle use |
| Gateway container memory | 64 MiB | Compose ceiling, not a reservation or measured idle use |
| Dashboard processes | 128 | Compose PID ceiling |
| Gateway processes | 64 | Compose PID ceiling |

`helix-privd`, Docker, AMP, and game-server containers are outside the two web
container memory ceilings. Server RAM must never be reported as Helix dashboard
RAM.

## Investigation budgets

These are tripwires for a future clean Ubuntu reference run. Exceeding one
requires investigation or a measured exception; meeting one does not prove
production readiness.

| Measurement | Budget | Conditions |
| --- | ---: | --- |
| Idle `helixd` proportional RSS | 30 MiB initially acceptable; 20 MiB desired | Release build, setup complete, no browser or optional work |
| Idle CPU | Effectively zero | Ten minutes after warmup with no requests |
| Warm readiness | 500 ms p95 | Healthy existing state, 30 starts |
| Cold readiness | 1 s p95 | New local-SSD state, 30 starts |
| Minimal loopback liveness | 5 ms p95 | 1,000 sequential requests |
| Authenticated read API | 25 ms p95 | Excludes password hashing and deliberately expensive work |
| Base background database writes | Zero | No state change or enabled sampler |
| Combined `helixd` and `helixctl` binaries | 20 MiB | Stripped release binaries; target recorded |

SQLite integrity, authentication work factors, request bounds, and recovery
behavior are not negotiable benchmark shortcuts.

## Historical development evidence

The 2026-08-27 production frontend build recorded:

| Measurement | Result | Budget |
| --- | ---: | ---: |
| Initial transfer | 55.9 KiB gzip | 75 KiB gzip |
| Initial JavaScript | 37.9 KiB gzip | 40 KiB gzip |
| Servers route | 23.2 KiB gzip | Lazy-loaded |
| Terminal route | 87.3 KiB gzip | Lazy-loaded; xterm is not paid for on other pages |
| All precompressed frontend files | 909,679 bytes raw / 237,682 gzip / 204,840 Brotli | Informational, not an initial-load budget |

These are transfer sizes from one locked production build, not browser-memory
or interaction-latency measurements. Both enforced initial budgets passed.

An early Windows development snapshot on 2026-08-26 recorded:

| Measurement | Result | Important limit |
| --- | ---: | --- |
| `helixd.exe` | 7.24 MiB | Foundation-era Windows binary, not current Linux installed size |
| `helixctl.exe` | 3.04 MiB | On-demand CLI; no resident cost |
| Idle working set | 18.71–18.72 MiB | One 30-second interval, not proportional RSS or a percentile |
| Idle process CPU | 0 ms reported | One 30-second interval; wakeups were not measured |
| `/healthz` | 0.283 ms p95 | 1,000 sequential loopback requests |
| Authenticated health | 0.374 ms p95 | 500 sequential loopback requests |
| First daemon readiness | 563.4 ms | One new-state sample including process-launch/polling overhead |

A later transport/recovery soak recorded 10,000 authenticated health requests
at 0.44 ms p95 and a ten-minute post-stress working set that fell from 25.37 MiB
to 22.19 MiB. It also exercised bounded overload, silent-connection expiry, an
unclean restart, database checks, and a verified state snapshot.

Those runs predate substantial broker, storage, native-server, network, package,
and marketplace work. They are retained as regression context only. They do not
describe current Linux RSS, broker cost, complete frontend size, or installed
storage.

## How to measure a current build

Record:

- exact revision and whether the tree is dirty;
- OS, kernel, architecture, CPU, RAM, storage, and power mode;
- Rust/Node versions, build profile, allocator, and target;
- configuration, enabled features, state size, browser connection, and other
  workloads;
- warmup, duration, sample count, tool, and raw result location; and
- p50/p95/p99, peak, or average as appropriate, never only the best result.

On Linux, separate RSS, proportional set size, private dirty memory, mapped
files, and page cache. Measure the unprivileged daemon, root broker, private
gateway, and each game server separately. After the browser disconnects, watch
CPU time, wakeups, context switches, and writes for at least ten minutes.

Frontend evidence records raw/gzip/Brotli size by initial and lazy chunk, usable
interaction, route transitions, long tasks, memory, mobile layout, keyboard
behavior, reduced motion, and error states.

## Game hosting

Helix does not execute Minecraft ticks or process player traffic. Performance
evidence for Helix must show bounded control-plane CPU, memory, I/O, API latency,
history retention, and UI behavior while the real game is loaded. The game's
player capacity requires a separate exact version/hardware/world/configuration
load test. See [Game Hosting Capacity](GAME-HOSTING-CAPACITY.md).

## Remaining reference gates

Still missing:

- clean supported-Ubuntu installed size and proportional RSS;
- broker and gateway idle/enabled marginal cost;
- repeated cold/warm startup distributions;
- long-running console retention and storage analysis pressure;
- native server, backup, marketplace, UFW, and AMP load/failure matrices;
- representative desktop/mobile browser timing; and
- performance under real small, busy, and saturation game workloads.

Until those exist, “lightweight” remains a design target backed by budgets and
limited development evidence, not a universal claim.
