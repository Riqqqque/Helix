# Performance Budget and Baseline

## Status

Helix does not yet have a reference Ubuntu baseline. Measurements from a Windows development host may be recorded below to catch obvious regressions, but they do not satisfy the Phase 0 performance gate. Reference numbers require a release build installed as the `helix` service user on a clean supported Ubuntu Server host with systemd and cgroup v2.

“Lightweight” is an earned description. A feature that is not enabled should create no timer, poll, connection, child process, large cache, or resident heavyweight runtime.

## Initial investigation budgets

These are engineering tripwires, not public guarantees. Exceeding one requires investigation and either a fix or a measured exception with conditions.

| Measurement | Founding budget | Conditions |
| --- | ---: | --- |
| Idle `helixd` proportional RSS | ≤30 MiB initially acceptable; ≤20 MiB desired; >40 MiB requires investigation and justification | Release build, setup complete, no browser requests, no optional modules |
| Idle CPU | Effectively zero; no periodic wakeup pattern without an enabled sampler | At least 10 minutes after warmup, one logical host, no requests |
| Warm readiness | <500 ms p95 | Existing healthy databases and static assets, 30 starts |
| Cold readiness | 1 s p95 | New empty data root on local SSD, includes schema creation, 30 starts |
| Minimal liveness API | 5 ms p95 | Loopback, 1,000 sequential requests after warmup |
| Authenticated cached/read-only API | 25 ms p95 excluding deliberately expensive password work | Loopback, release build, 1,000 sequential requests |
| Initial frontend transfer | 75 KiB gzip total | HTML, CSS, and JavaScript needed for the first usable route |
| Initial JavaScript | 40 KiB gzip | First usable route; code-split optional tools excluded |
| Combined release binaries | 20 MiB | Stripped `helixd` and `helixctl`, target architecture recorded |
| Base background database writes | Zero without a state change or enabled sampler | Ten-minute idle observation |

The budgets will be revised from representative low-end and modern hardware evidence. SQLite durability settings, integrity checks, input validation, and recovery behavior are not negotiable benchmark shortcuts.

## Measurement protocol

Every result records:

- Helix revision and dirty-state identifier;
- exact OS, kernel, architecture, CPU, memory, storage medium, and power mode;
- Rust, compiler target, build profile, allocator, Node, and frontend versions;
- configuration, enabled features, database size, and whether the run is cold or warm;
- sample count, warmup, duration, command or tool, and raw result location;
- median, p95, peak, or average as appropriate, not only the best run;
- background workload and whether a browser was connected.

### Memory

On Linux, record RSS, proportional set size where available, private dirty memory, mapped-file contribution, and peak during startup and first dashboard load. Distinguish daemon memory from browser memory and page cache.

### CPU and wakeups

After startup and one warm dashboard request, disconnect the browser and observe at least ten minutes. Record process CPU time, normalized CPU percentage, context switches, wakeups, and unexpected I/O. Repeat with each optional sampler enabled so its marginal cost is visible.

### Startup

Measure from process creation to a successful readiness response. Cold runs use a new data root and include migration; warm runs use a cleanly shut down healthy installation. Record bind conflicts, unclean checks, and missing assets separately rather than hiding them in an average.

### API latency

Use loopback and a release build. Report response-size and p50/p95/p99 for minimal liveness, authenticated health, host overview, and a representative SQLite read. Bound client concurrency, then separately test the configured overload behavior. Password hashing is benchmarked as a security operation, not mixed into ordinary API latency.

### Frontend

Record raw, gzip, and Brotli sizes by artifact and route. Measure first content, usable interaction, route changes, long-task count, and memory on desktop and a representative mobile viewport. A fast synthetic build does not replace live keyboard, responsive, reduced-motion, and error-state checks.

### SQLite

Measure ordinary reads, durable critical writes, metrics batches, checkpoints, online snapshots, and migrations against realistic database sizes. Keep critical `synchronous=FULL`; metrics and critical state remain separate result sets.

## Baseline results

No reference Ubuntu results have been recorded yet.

### Non-reference Windows development snapshot — 2026-08-26

This snapshot is useful only for catching obvious regressions. It was taken on
Windows 11 x64 with 16 logical processors and 31.1 GiB of RAM, using Rust/Cargo
1.94.1 targeting `x86_64-pc-windows-msvc`, Node 24.12.0, and npm 11.6.2. The
Rust binaries used the checked-in release profile with thin LTO and stripped
symbols. This snapshot predates the first commit, so its identity is the
founding working tree rather than a reviewable Git revision.

| Measurement | Result | Conditions and limits |
| --- | ---: | --- |
| `helixd.exe` | 7,568,384 bytes (7.22 MiB) | Release build, all workspace features |
| `helixctl.exe` | 2,857,472 bytes (2.73 MiB) | Release build, all workspace features |
| Combined release binaries | 10,425,856 bytes (9.94 MiB) | Windows PE files; not Linux installed size |
| Compiled frontend | 91,465 bytes raw; 24,733 gzip; 21,838 Brotli | HTML, CSS, and JavaScript for the authenticated base route |
| First daemon readiness | 563.4 ms | One sample on a new data root after `helixctl setup-token` initialized state; includes `Start-Process` and a 25 ms polling interval, so it is neither the 30-run warm p95 nor a pure cold-schema result |
| `/healthz` | p50 0.202 ms; p95 0.283 ms; p99 0.322 ms | 1,000 sequential requests through one .NET `HttpClient`, release daemon, loopback |
| Authenticated `/api/v1/health` | p50 0.280 ms; p95 0.374 ms; p99 0.405 ms | 500 sequential requests with a valid cookie and CSRF proof through the same client |
| Idle process | 0 ms CPU over 30 seconds | No browser and no requests; one interval, not the required ten-minute wakeup study |
| Idle working set | 18.71 MiB to 18.72 MiB | Private bytes stayed at 7.86 MiB; process peak working set since launch was 37.46 MiB |

The live flow had one owner, completed setup/authentication activity, two storage
mounts, two network interfaces, and no optional modules. The latency client and
Windows scheduler are part of these values. Raw E2E state was deliberately
deleted after the canary scan, so this summary is not suitable as a retained
reference benchmark record.

The comparable size, RSS, and API samples are below the initial investigation
budgets, but one sample cannot establish a percentile or a public performance
claim. The startup number is not comparable to either required startup protocol.
Ubuntu installed size, proportional RSS, CPU wakeups, repeated startup,
authenticated SQLite reads/writes, dashboard timing, and durability-preserving
database latency remain **BLOCKED**.

## Regression policy

- Release validation records binary and frontend sizes; CI artifact/size reporting remains to be implemented before it can act as a regression gate.
- A change that adds a background task must include its disabled and enabled cost.
- A dependency that materially changes startup, RSS, binary size, or initial bundle size needs justification.
- A result outside a budget blocks the relevant milestone until fixed or documented as a deliberate exception.
- Raw or unfavorable measurements are retained; benchmark conditions are never changed silently to improve a headline.
