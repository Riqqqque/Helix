# ADR 0002: SQLite Durability Domains

- **Status:** Accepted
- **Date:** 2026-08-25

## Context

Helix is primarily a single-host control plane with modest transactional concurrency. It needs durable users, permissions, instance definitions, policies, jobs, backup catalogs, and configuration revisions. It also needs high-volume historical metrics where losing the newest samples is inconvenient but not equivalent to losing an owner account or game definition.

Using identical durability and retention for both classes either weakens critical state for benchmark numbers or makes telemetry needlessly expensive. A database server would add resident memory, operational dependencies, authentication, backup, and upgrade burden to the base product. Putting worlds, archives, logs, or binaries in a relational database would create another bottleneck and failure domain.

SQLite calls are synchronous at the engine boundary even when invoked from asynchronous Rust. An unbounded connection pool or direct blocking on Tokio executor threads would undermine latency and memory goals.

## Decision drivers

- Correctly committed critical changes survive ordinary crashes and power loss on a functioning storage stack.
- Metrics cannot take down authentication or administration.
- No database server is required.
- Connection and memory use are small and bounded.
- Migrations and live snapshots are recoverable.
- Database operations do not block async executor threads.
- Storage policy is explicit and testable on every connection.

## Options considered

### One SQLite database with one durability policy

This is simple to open and back up, but joins telemetry churn, WAL growth, retention, corruption handling, and latency to critical configuration. Tuning it toward metrics weakens important state; tuning it toward state imposes avoidable write cost. Rejected.

### Separate SQLite databases by consequence

Use a critical-state database with strong durability and a metrics database with bounded, recoverable loss. Large data remains on the filesystem. This preserves a small deployment while isolating workload and failure behavior. Chosen.

### External database server

PostgreSQL or another service could offer more concurrent writers and remote operation. Helix does not need that concurrency in its single-host foundation, and the service would violate the base dependency and idle-resource goals. Future remote nodes should use a protocol to the owning control plane rather than expose its local database. Rejected.

### In-memory metrics only

This has minimal write cost but loses useful history after every restart. It remains appropriate for the highest-frequency live window, not all historical metrics. Rejected as the sole metrics store.

## Decision

### Critical state domain

`/var/lib/helix/state/helix-state.db` owns durable control-plane metadata in production. The private directory uses mode `0700` and database and backup files use `0600`. Every connection explicitly sets and verifies:

```sql
PRAGMA journal_mode=WAL;
PRAGMA synchronous=FULL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=<bounded measured value>;
```

WAL mode is accepted only when SQLite reports that it took effect. Foreign keys are enabled outside a transaction on every connection and checked. Tables use deliberate `NOT NULL`, `CHECK`, unique, foreign-key, and index constraints.

Writes are short and use prepared statements. The initial concurrency model is one serialized write path and a small bounded read capacity. Long reads and transactions are not allowed to pin the WAL indefinitely. SQLite work runs on a dedicated database worker or an explicitly bounded blocking executor; it does not run directly on a Tokio core thread.

Critical state includes:

- users, roles, capabilities, sessions, and token metadata;
- instance, module, storage-pool, schedule, and port definitions;
- jobs, operation intents, locks, and audit metadata;
- layouts, themes, policies, and versioned configuration metadata;
- backup catalog and restore-confidence metadata;
- future node identity and compatibility metadata.

Recoverable secret ciphertext may be referenced or stored here, but the master key does not live in this database. Passwords and bearer credentials are represented by verification hashes, not reversible plaintext.

### Metrics domain

`/var/lib/helix/metrics/helix-metrics.db` uses its own `0700` directory, `0600` database file, connection manager, and schema:

```sql
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=<bounded measured value>;
```

Samples are batched, rolled up, and removed according to bounded retention. The high-frequency live window is primarily a bounded in-memory ring. Persistent collection adapts to active use rather than writing every host metric every second forever.

Metrics corruption or unavailability produces a health event, preserves a forensic copy where safe, and recreates the metrics store. It does not prevent critical-state access. Metric labels and dimensions are constrained so they cannot become an unbounded cardinality or sensitive-data store.

### Filesystem domains

Worlds, saves, game files, archives, logs, configuration payloads too large for sensible rows, and content-addressed downloads remain on the filesystem. SQLite stores stable identifiers, metadata, hashes, and operation state.

SQLite and filesystem actions cannot share a transaction. Important cross-resource operations use a durable ledger, same-filesystem staging, atomic publish steps, and startup reconciliation as defined in `docs/STORAGE.md`.

### Checkpoints

The initial automatic checkpoint policy is 1,000 pages for both databases, matching the storage contract. The initial journal-size limits are 16 MiB for critical state and 64 MiB for metrics. Effective settings, WAL size, checkpoint duration, and blocked-checkpoint results are observable. These values are starting policies, not performance claims or guarantees that the WAL can never temporarily exceed the limit.

Non-blocking passive checkpoints may run after backups or when observed WAL growth exceeds its normal envelope. Full, restart, or truncate checkpoints require a bounded quiet maintenance point. A latency-sensitive request must not unexpectedly perform an aggressive checkpoint.

### Migrations

Migrations are immutable and ordered. Startup refuses a schema newer than the
running binary understands. Before catching an existing non-empty critical
database up through one or more pending steps, Helix:

1. prevents normal writers from entering and fully validates the source;
2. completes a `TRUNCATE` WAL checkpoint and hashes the durable source file;
3. reuses a rollback snapshot only through an exact source-schema,
   target-schema, source-SHA alias whose content-addressed target revalidates;
4. otherwise creates one consistent snapshot through SQLite's online backup
   API, verifies and durably publishes it, and rechecks the source identity;
5. applies transactional work where supported;
6. checks the resulting schema, migration history, foreign keys, and integrity;
7. reopens through the normal path and verifies effective PRAGMAs.

Identical retries do not recopy the same source. Changed source content produces
a distinct rollback object, while altered aliases or targets fail closed.
Deterministic current staging files and at most 16 legacy partials are reconciled
per open so restart cleanup remains bounded.

Copying only the main file of an open WAL database is not a backup. Migrations with filesystem effects use the operation ledger and explicit recovery steps.

### Integrity and corruption

Bounded quick and foreign-key checks are routine; full integrity checks run on an explicit schedule or after risk indicators. An unclean shutdown triggers full validation before startup session cleanup or critical writes.

Critical-state corruption fails closed. Before destructive repair, Helix preserves all database components and diagnostic context when the storage medium permits it. Metrics may be recreated only after preserving a forensic copy or an explicit operator waiver.

### Durability limits

`synchronous=FULL` expresses the correct SQLite policy but cannot force faulty hardware, a lying storage controller, unsafe mount options, or an incompatible network filesystem to honor flushes and locks. Critical databases default to tested local filesystems. Helix documents the storage stack used for its claims and does not promise power-loss durability on an untested backend.

## CIA consequences

### Confidentiality

Separate files permit restrictive permissions and keep metrics out of ordinary state snapshots when appropriate. Separation is not encryption. Sensitive values require application-layer authenticated encryption or one-way hashing, logs require redaction, and filesystem permissions remain mandatory.

### Integrity

Critical state keeps full synchronous durability, constraints, checks, migration snapshots, and short transactions. Metrics cannot become authoritative for desired state or authorization. Cross-resource ledgers prevent a database row from falsely proving a filesystem action completed.

### Availability

Metrics failure is isolated, buffers and retention are bounded, and the base system needs no database service. SQLite still has one writer per database; long transactions, disk-full conditions, WAL growth, or filesystem failure can reduce availability and require protective modes.

## Consequences

Positive:

- strong critical-state policy without paying the same cost for every metric;
- independent metrics retention and corruption response;
- small installation and backup surface;
- straightforward local inspection and recovery tooling;
- bounded connection strategy suited to expected concurrency.

Costs:

- two schemas, migration histories, connection managers, backup policies, and health states;
- dashboards may combine data with different timestamps and availability;
- cross-domain reports cannot rely on one SQL transaction;
- careful async/blocking integration is required;
- corruption and disk-pressure fixtures are more involved than ordinary unit tests.

## Validation

Implementation is not considered tested until Linux integration and fault tests demonstrate:

- effective PRAGMAs on every connection;
- concurrent read/write busy behavior and bounded latency;
- no SQLite work blocks Tokio core threads under load;
- online snapshots during concurrent writes;
- migration from every supported historical schema and recovery after interruption;
- unclean shutdown and process kill around commits and checkpoints;
- disk-full, read-only, permission, WAL-growth, and slow-reader behavior;
- critical-state corruption fails closed;
- metrics corruption degrades only telemetry;
- reference API/SQLite latency with `FULL` durability still enabled.

## Revisit triggers

Revisit the connection model if measured contention violates documented latency objectives, not merely because a larger pool is available. Revisit SQLite only if real workloads require concurrency or scale it cannot safely provide. Any replacement must improve measured product behavior enough to justify a new base dependency, migration path, operational burden, backup strategy, and recovery model.
