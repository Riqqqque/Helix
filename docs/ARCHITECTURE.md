# Helix Architecture

## Status and scope

This document defines the founding boundaries for Helix. It is a design contract, not evidence that the described components have been implemented or validated. Where implementation and this document disagree, the discrepancy must be resolved explicitly and recorded in an ADR; neither side should silently drift.

Helix is a single-host, local-first Linux control plane. Remote nodes are a future compatibility concern, not part of the foundation. The base installation must not require a database server, container engine, reverse proxy, language runtime for games, or JavaScript runtime.

## Architectural invariants

1. `helixd` is a control plane, not the parent process of managed services.
2. Managed workloads run under their independent systemd/Docker/AMP runtime and
   continue through a daemon or frontend failure.
3. `helixd` does not run as root.
4. Privileged behavior is exposed as narrow typed operations, never as arbitrary shell execution.
5. Critical state, disposable telemetry, large game data, caches, logs, and secrets have different storage and durability policies.
6. Optional features do not create resident memory, polling, database, or network cost while disabled.
7. The API and persistent services are authoritative. Frontend state is a projection, never the source of truth.
8. Long-running and cross-resource operations are explicit jobs with durable intent, observable progress, cancellation rules, and restart reconciliation.
9. Large data paths are streamed with bounded buffers and backpressure.
10. Security, performance, and recoverability claims require tests or measurements.

## Context

```text
Browser
   |
   | HTTP + SSE/WebSocket
   v
helixd (unprivileged) ---- helix-state.db
   |       |              helix-metrics.db
   |       +------------> read-only /proc, /sys, system APIs
   |
   +-- typed local IPC --> helix-privd (root, socket activated)
   +-- authenticated IPC -> helix-terminald (optional, one Linux user)
   +-- job dispatch ----> helix-worker (one shot, constrained)
   +-- optional IPC ----> helix-strandd (sandbox host)
   +-- systemd control -> independent managed workloads

helixctl --> local API/IPC; it does not bypass authorization by default
```

The arrows describe allowed interactions, not a promise that every path exists today.

## Process boundaries

### `helixd`

`helixd` is the only continuously resident process required by the base installation. It owns:

- HTTP API and static asset delivery;
- authentication, authorization, and session enforcement;
- orchestration and policy decisions;
- critical state access;
- lightweight, demand-aware host observations;
- event publication and job coordination;
- startup reconciliation and health reporting.

It must not execute arbitrary commands, deserialize untrusted native plugins, perform large archive operations on async executor threads, or become the lifetime owner of a managed game process.

### `helixctl`

`helixctl` is an on-demand administrative client. It presents status, setup,
diagnostics, backup, recovery, import/export, repair, and update operations. It
also exposes installation-independent developer commands such as preview Strand
scaffolding and validation. Administrative commands should use the same domain
operations and authorization policy as the API wherever possible. Possession of
a local binary is not itself authorization.

Offline recovery commands may need direct file access. Those commands must be explicit, require the daemon to be stopped when necessary, create forensic copies before destructive repair, and state which invariants they temporarily bypass.

### `helix-privd`

`helix-privd` is the implemented minimal privileged broker, started as a
separate root systemd service in the checked container deployment. Its protocol
is an allowlist of versioned request and response types such as managing a
specific Helix-owned container/rule/path or applying a validated host change. It must:

- authenticate the calling peer using local operating-system credentials;
- authorize the exact operation and target;
- revalidate paths, identifiers, arguments, and expected state independently;
- reject symlink and path traversal escapes;
- apply timeouts and output limits;
- produce redacted audit events;
- expose no `run_command(string)` equivalent.

The broker must not trust validation performed only by `helixd`.

### `helix-terminald`

`helix-terminald` is an optional, separately deployed real-PTY bridge. It runs
as one configured non-root Linux account, uses a socket group distinct from the
root broker, and checks the connecting dashboard UID through Linux
`SO_PEERCRED`. The API requires a fresh dashboard-password proof and a one-use
session-bound ticket for every WebSocket. The service has bounded frames and
sessions, clears inherited environment values, kills the PTY on disconnect, and
does not log terminal input/output. It is not a privileged broker operation.

### `helix-worker`

`helix-worker` is a planned one-shot execution target for memory-, CPU-, I/O-, or failure-heavy jobs. systemd transient units/scopes should constrain memory, CPU weight, I/O priority, runtime, and cancellation. Workers receive an immutable job identifier and bounded capability description rather than broad daemon credentials. Results are committed through the job protocol; a worker crash must not corrupt unrelated daemon state.

### `helix-strandd`

`helix-strandd` is a planned optional extension host. A WebAssembly runtime or other heavyweight sandbox must not be linked into the base daemon merely for future use. Strands receive declared capabilities, resource limits, versioned APIs, and namespaced storage. They cannot call the privileged broker directly.

### Managed workloads

Current Helix-native Minecraft servers are separate Docker containers. Adopted
host services remain exact systemd units, while AMP stays its own manager.
Helix records stable opaque identities and observes the owning runtime as
authority; names and paths never derive directly from display names. Restart
policy and crash-loop protection are part of each workload definition.

Helix is outside the game's simulation and player network path. Hosting detail
is demand-loaded through bounded pages and streams; no dashboard viewer means no
per-player polling, retained fanout queue, or telemetry writer. Resource changes
stay inside an operator-approved host/instance envelope. The complete capacity
contract and evidence boundary are in
[Game Hosting Capacity](GAME-HOSTING-CAPACITY.md).

## Crate boundaries

The foundation workspace uses these crates:

| Crate | Owns | Must not own |
| --- | --- | --- |
| `helix-core` | Domain identifiers, state machines, policy types, shared errors, and trait contracts | Axum routes, SQL, Linux parsing, frontend assets |
| `helix-auth` | Canonical identity, prospective-password policy, bounded Argon2id verification, and opaque-token primitives | HTTP cookies, SQL, async execution, or authorization policy |
| `helix-config` | Typed process configuration, validation, defaults, and filesystem path policy | Runtime state or arbitrary environment access throughout the codebase |
| `helix-state` | Critical SQLite schema, migrations, repositories, transaction boundaries, and integrity checks | HTTP concerns, host polling, game files |
| `helix-secrets` | Redacted master-key credentials, authenticated record envelopes, and closure-only plaintext access over `helix-state` | Credential provisioning, HTTP responses, config/env/CLI key loading, rotation orchestration |
| `helix-strand-kit` | Non-executing preview project scaffolding, strict manifest parsing, and author-facing validation summaries | Package installation, signature trust, capability grants, host calls, sandboxing, or extension execution |
| `helix-system` | Narrow read-only Linux host discovery and metric snapshots | Privileged mutation, persistent polling loops, policy decisions |
| `helix-api` | Versioned HTTP contracts, routing, extraction, middleware, and response mapping | Direct SQL strings, root operations, business invariants defined only in handlers |
| `helix-privd` | Closed broker protocol plus Linux host, storage, network, native-server, package, and integration operations | General shell RPC, arbitrary unit/binary execution, frontend state |
| `helix-terminal` | Framed bounded real-PTY protocol and non-root Linux bridge | Root broker calls, command/output persistence, browser authorization policy |
| `helixd` | Dependency wiring, process lifecycle, task supervision, graceful shutdown, and asset serving | Reusable domain logic |
| `helixctl` | CLI presentation and calls into supported administrative interfaces | A second, inconsistent implementation of control-plane policy |

Planned process crates such as `helix-worker` and `helix-strandd` are added only
when their phase begins. Dependencies flow inward toward `helix-core`; cyclic
dependencies are not allowed. Cross-crate data should use purpose-built types
instead of unversioned JSON blobs.

`helix-system` returns snapshots or subscriptions requested by an orchestrator. It does not start global one-second polling on construction. Read-only does not mean harmless: filesystem reads must be bounded, parsing defensive, and blocking work kept off Tokio executor threads.

## Privilege boundary

The service account owns Helix state and the instances it is allowed to manage. Root-owned operating-system changes cross the local privileged protocol. The normal daemon must be unable to expand its own privilege by choosing an executable, raw command, arbitrary unit name, or arbitrary filesystem path.

The boundary is enforced at both ends:

- callers express a typed intent and supply expected revisions;
- the broker maps identifiers to policy-controlled targets;
- the broker checks peer identity, authorization, path containment, and current state;
- results are structured and size bounded;
- an audit record identifies actor, action, target, result, and correlation ID without storing secrets.

## Data and database boundaries

### Critical state

`helix-state.db` contains users, roles, sessions metadata, instance definitions, enabled modules, jobs, audit metadata, backup catalog entries, storage pools, schedules, layouts, policies, and configuration revisions. It uses WAL mode, full synchronous durability, foreign keys, short transactions, controlled checkpointing, and pre-migration backups.

SQLite work is synchronous at the engine boundary. It runs through a deliberate bounded database execution layer rather than blocking async executor threads. The initial design favors one serialized write path and a small bounded read capacity; measured contention may justify refinement, not an oversized pool.

Authentication/session audit rows use fixed local retention: a 90-day window,
a protected newest floor of 1,024 rows, a 50,000-row steady-state ceiling, and
at most 256 deletions in one audited-write or startup transaction. Cleanup is
append-only from ordinary application code and temporarily opens its delete
guard only inside the same immediate transaction that restores it. Export,
holds, hash chaining, and off-host evidence are separate Chronicle work.

Cancelling an HTTP future does not cancel work already dispatched to a blocking
thread. Every state/password blocking closure therefore holds a shared task
tracker guard for its actual lifetime. `helixd` drains Axum, waits for that
tracker to become idle without a lost-wakeup race, and only then writes the
clean-shutdown marker. Both drains share a 20-second deadline, leaving systemd
10 seconds of hard-stop margin. Expiry forces process termination with the run
left unclean, so startup performs the stronger recovery validation.

### Metrics

`helix-metrics.db` is a separate failure and durability domain with bounded retention, rollups, batched writes, WAL mode, and normal synchronous durability. Corruption or unavailability of metrics must degrade history and raise a health event without preventing authentication or administration. High-frequency live samples belong primarily in a bounded in-memory ring buffer.

### Filesystem and secrets

Worlds, saves, server binaries, archives, logs, and caches do not belong in SQLite. The implemented portable secret boundary uses XChaCha20-Poly1305 per-record DEKs, wraps each DEK under a master key stored separately from the database, and authenticates stable identity and revisions. It exposes no HTTP plaintext path. Production credential delivery, rotation, and independent recovery remain outside the implemented boundary. Details and limitations are in `docs/SECURITY.md` and [ADR 0005](adr/0005-recoverable-secret-storage.md).

### Multi-resource consistency

SQLite and filesystem changes do not share an ACID transaction. Important operations use a durable operation ledger:

1. record validated intent and expected revisions;
2. stage files on the same filesystem as their destination;
3. flush and atomically rename where durability requires it;
4. commit corresponding metadata;
5. mark completion;
6. reconcile incomplete entries on startup.

Recovery chooses an explicit finish, rollback, restore, or intervention state. It never silently assumes a partially applied operation succeeded.

## Filesystem layout

The planned package layout follows Linux conventions:

| Path | Purpose |
| --- | --- |
| `/usr/bin/helixd` | Daemon executable |
| `/usr/bin/helixctl` | CLI executable |
| `/usr/share/helix/web/` | Immutable compiled frontend assets |
| `/etc/helix/helix.toml` | Administrator-owned non-secret configuration |
| `/var/lib/helix/state/helix-state.db` | Critical state database |
| `/var/lib/helix/metrics/helix-metrics.db` | Replaceable metrics database |
| `/var/lib/helix/instances/<uuid>/` | Instance server, data, config, log, temporary, and metadata areas |
| `/var/lib/helix/keys/` | Restrictively permissioned fallback key material, subject to the security design |
| `/var/cache/helix/downloads/<algorithm>/<digest>` | Bounded content-addressed download cache |
| `/run/helix/` | Runtime sockets and transient process state |

Private state, metrics, and key directories use mode `0700`; database and database-backup files use `0600`. Package integration tests must verify the effective owner and mode after creation, migration, backup, upgrade, and restore rather than relying on the process umask.

Storage pools can place instance or backup data on other mounted filesystems. The state database stores stable pool identifiers and validated roots. Display names never form trusted paths. Logs should use journald first; game log retention is separately bounded.

Development path overrides must be explicit and must never change production defaults silently.

## Event architecture

Domain actions create typed events after their authoritative state transition commits. Events serve three different purposes:

- in-process notification through bounded Tokio channels;
- live client updates through Server-Sent Events by default;
- durable security/audit or job history stored in the appropriate state tables.

An in-memory event is not an audit record. A reconnecting client refetches authoritative resources and may use an event cursor where supported. Slow consumers must not create unbounded queues: coalescible metrics may be replaced by a newer sample, while security and job state transitions use durable records. Bidirectional WebSockets are reserved for interactions such as a console where SSE is insufficient.

## Job architecture

Any operation that may outlive one request is a job. A job records:

- type, actor, target, and redacted parameters;
- lifecycle state and monotonic progress where meaningful;
- idempotency key and expected resource revision;
- required resource locks;
- cancellation and timeout policy;
- attempt history and resumability classification;
- result or structured failure.

The API returns `202 Accepted` and a job resource for asynchronous work. The coordinator prevents incompatible work such as restore during backup. On startup, it reconciles running jobs according to their type: resume only when designed to be resumable, otherwise roll back or require intervention. A database row alone is not proof that an external side effect occurred.

## Module lifecycle

Every optional module has explicit disabled, starting, running, degraded, and stopping states. Enabling registers routes, jobs, collectors, and UI capabilities through defined interfaces. Disabling cancels timers, drains work, closes resources, and unregisters background activity. Metrics collection adapts to demand rather than using a permanent global polling interval.

## Extension and game-definition boundaries

Sequences are declarative, versioned, signed or checksum-verified data. Parsing and validation happen before an action plan is produced. Install actions are a closed typed set; Sequences cannot inject a shell script or construct arbitrary commands.

Strands are untrusted by default. Their manifests declare capabilities and limits, and the optional host enforces them. Trusted native sidecars require a separate review because native code is not sandboxed merely by calling it an extension.

Runtime backends implement a shared lifecycle contract. Native systemd execution is first. Container support is optional and does not become a base dependency. Game support is data-driven where safe, but lifecycle claims require real game-specific tests.

## Backup and recovery boundary

Vault coordinates backup policy and catalogs, but a backend owns the actual repository format. Backends stream data and report verifiable manifests and checksums. State database snapshots use SQLite's supported online-backup mechanism; copying a live WAL database is not accepted as a consistent snapshot.

A successful upload is not a verified backup, and an archive that opens is not a verified restore. Restore drills stage into a temporary location, validate manifests and checksums, validate the recovered structure, clean up, and record the result. Destructive repair first preserves forensic evidence when space and safety permit.

## API boundary

The HTTP API is versioned under `/api/v1`. Handlers authenticate, authorize, validate, and translate transport types, then call domain operations. They do not embed raw SQL, shell commands, or filesystem paths. Mutations support resource revisions and idempotency where retries could duplicate work. Details are in [API](API.md).

## Frontend boundary

The frontend is a static Preact and TypeScript application built with Vite. Node.js is a build dependency only. The packaged assets are served by `helixd`; the production host does not install Node.

The browser holds view and editing state, not authoritative server state. Route-level code splitting keeps charts, terminals, editors, game-specific views, and Strand UI out of the initial shell. A component library, chart engine, editor, or icon collection must justify its payload and accessibility cost. The provisional framework decision and its validation gate are recorded in [ADR 0003](adr/0003-frontend-framework.md).

## Source-of-truth map

| Concern | Authority |
| --- | --- |
| User, role, instance, policy, job definition | Critical state database |
| Service lifecycle and unit state | systemd/Linux |
| Game files and saves | Filesystem |
| Backup object availability | Vault backend plus verified catalog |
| Host counters | Kernel/system interfaces, sampled by `helix-system` |
| Historical metrics | Metrics database |
| Current edit form | Browser only until a revision-checked API mutation succeeds |

Conflicts are surfaced; the UI must not invent a successful state because a button was clicked.

## CIA analysis by boundary

| Boundary | Confidentiality | Integrity | Availability |
| --- | --- | --- | --- |
| API/session | Secure cookies or scoped tokens, redaction, minimal public responses | Server-side authorization, CSRF/origin checks, revision checks | Request limits, timeouts, graceful degradation |
| Critical state | Restrictive permissions; secrets encrypted separately | FULL synchronous SQLite, foreign keys, migrations, integrity checks | Online snapshots, bounded contention, startup recovery |
| Metrics | Avoid sensitive labels and filenames | Typed samples and schema validation | Failure is isolated; retention and buffers are bounded |
| Privileged broker | Local socket and peer authentication; no secret output | Typed allowlist, target revalidation, audit trail | Socket activation, timeouts, no dependency for read-only UI |
| Host terminal | One-use session/password proof; distinct socket group; peer UID | Non-root PTY, bounded frames, no root-broker path | Optional service, two-session cap, disconnect cleanup |
| Managed workloads | Per-instance permissions and secret scoping | Stable IDs, systemd definitions, controlled changes | Independent lifetime, resource limits, crash-loop protection |
| Workers | Minimum job capability and sanitized logs | Immutable inputs, checksums, staged output | Resource limits, cancellation, resumable/reconciled jobs |
| Strands | Capability-scoped APIs and namespaced storage | Signed/verified packages and validated messages | Optional host, quotas, crash isolation |
| Frontend | No durable secret storage; avoid sensitive cache | Server remains authoritative; CSP and asset integrity policy | Static shell, lazy features, API reconnection states |
| Backups | Encryption and separated recovery material | Manifests, checksums, authenticated encryption | Multiple destinations, retention, tested restoration |

Helix cannot protect secrets from an attacker who already controls root or the running process. It can limit persistence, blast radius, accidental disclosure, and damage from lower-privilege actors. Detailed threat assumptions belong in `docs/SECURITY.md`.

## Failure containment

- A frontend failure leaves APIs and managed workloads running.
- A metrics failure disables or degrades history, not critical state.
- A Strand failure affects only that extension host and its in-flight calls.
- A worker failure leaves a reconcilable job and staged data, not a half-declared success.
- A daemon restart reconnects to systemd-managed workloads rather than relaunching them blindly.
- A disk-space emergency blocks optional high-write work before consuming reserved headroom.
- A malformed or oversized external input is rejected before unbounded allocation or extraction.

These are required properties. Fault-injection and Linux system tests are still needed before they can be claimed as observed behavior.

## Known limitations and open decisions

- No supported Ubuntu versions or packaging matrix have been validated yet.
- Linux systemd, cgroup v2, file-permission, upgrade, rollback, and power-loss behavior require Linux test hosts.
- TLS deployment modes, trusted proxy policy, and first-run remote access need implementation and security validation.
- Portable master-key checking and encrypted record storage exist, but Ubuntu systemd credential integration, rotation, and recovery-key handling remain unimplemented and unvalidated.
- The privileged protocol, worker protocol, Sequence schema, Strand ABI, and remote-node protocol are not stable.
- SQLite and frontend choices have architectural approval but still require measured implementation evidence.
- Multi-node operation is deliberately deferred; the current design must not imply distributed consensus.

## Related decisions

- [ADR 0001: Component and process boundaries](adr/0001-component-boundaries.md)
- [ADR 0002: SQLite durability domains](adr/0002-sqlite-durability-domains.md)
- [ADR 0003: Frontend framework](adr/0003-frontend-framework.md)
- [ADR 0004: Owner password and session authentication](adr/0004-owner-authentication.md)
- [ADR 0005: Recoverable-secret envelope storage](adr/0005-recoverable-secret-storage.md)
