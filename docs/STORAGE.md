# Storage and Data Integrity

## Status

This document defines Helix's founding storage contract. The foundation
currently implements a narrow SQLite slice in `crates/helix-state`: version-3
critical state and version-1 metrics schemas, verified connection PRAGMAs, an
installation/clean-shutdown marker, an operation ledger, owner/session security
state, encrypted-secret metadata, integrity and semantic checks, verified
migration and online snapshots, and metrics corruption
preservation/recreation. `crates/helix-secrets` implements the portable
authenticated-encryption boundary over that schema. Portable tests exercise
those primitives on the Windows development host.

That slice is not the full contract and has not passed Linux power-loss,
disk-full, concurrency, filesystem-race, credential-provisioning, key-rotation,
or restore drills. There is no general atomic configuration writer, low-disk
guard, production master-key delivery, independent secret recovery, Vault, or
state-database recovery workflow. `PROGRESS.md` must cite code and test evidence
before any additional item here is described as complete.

The storage design has four goals:

1. important state survives process crashes and power loss;
2. telemetry and caches cannot take the control plane down;
3. interrupted cross-filesystem work has an explicit recovery path; and
4. untrusted names and paths never choose trusted filesystem locations.

## Durability domains

Data is separated by consequence, not convenience.

| Domain | Authoritative source | Durability | Loss policy |
| --- | --- | --- | --- |
| Critical state | `helix-state.db` | SQLite WAL, `synchronous=FULL` | A committed change must survive a correctly functioning storage stack and power loss. Corruption fails closed. |
| Metrics | `helix-metrics.db` | SQLite WAL, `synchronous=NORMAL` | Recent samples may be lost after power loss. Corruption must not disable core administration. |
| Game and service data | Filesystem under a stable instance ID | Backend-specific, crash-safe Helix-owned writes | Never silently replace or discard. Application-consistent backup hooks are required. |
| Config revisions | State DB metadata plus immutable filesystem payloads where large | Same as critical state | Keep enough information to preview, roll back, and reconcile an interrupted write. |
| Secrets | Versioned ciphertext in state DB; master key outside it | Critical | Plaintext is never an accepted at-rest format. Losing keys is a documented recovery event. |
| Operation staging | Same storage pool as the target | Recoverable temporary state | Safe to resume, roll back, quarantine, or explicitly abandon. Never ambiguous. |
| Logs | journald for Helix; bounded per-instance logs | Best effort, bounded | Logging failure is visible but must not consume emergency free space indefinitely. |
| Cache/downloads | Content-addressed cache | Reproducible and bounded | May be evicted after use-count and in-progress leases are checked. |
| Runtime state | `/run/helix` | Ephemeral | Rebuilt after boot; never the sole copy of durable state. |

Large worlds, saves, archives, console history, and binary packages do not
belong in SQLite.

## Filesystem layout

The target Linux layout is:

```text
/etc/helix/
  helix.toml                 # root-owned configuration, no recoverable secrets

/var/lib/helix/
  state/helix-state.db
  metrics/helix-metrics.db
  keys/                      # fallback protected key material; excluded from ordinary exports
  instances/<instance-uuid>/
    server/
    data/
    config/
    logs/
    cache/
    tmp/
    metadata/
  operations/<operation-uuid>/
  vault/                     # local Vault data when explicitly configured

/var/cache/helix/
  downloads/<algorithm>/<digest>

/run/helix/
  helix-privd.sock
  workers/
```

The exact package-created ownership and modes must be integration-tested. The
baseline is a dedicated, unprivileged `helix` service account; `0700` for
private state/key directories; `0600` for databases and fallback key files;
and `0640 root:helix` for non-secret system configuration. Per-instance game
processes should use distinct service identities where practical. Helix must
not solve access problems by recursively making trees world-readable or by
broadly changing ownership.

Display names, hostnames, usernames, imported filenames, and game names are
metadata only. Stable UUIDs select instance and operation directories.

## Trusted path resolution

Every filesystem operation starts from an already opened, allowlisted root
directory. On Linux, prefer `openat2(2)` with appropriate `RESOLVE_BENEATH`,
`RESOLVE_NO_MAGICLINKS`, and, where required, `RESOLVE_NO_SYMLINKS` constraints.
A compatibility fallback must walk components with directory file descriptors
and no-follow semantics. A check-then-open string-path sequence is not an
acceptable fallback because it is vulnerable to races.

Rules for untrusted paths:

- reject absolute paths, empty components where ambiguous, `.` and `..`;
- reject NUL and platform-invalid components before reaching a system call;
- do not follow symlinks, hard links, magic links, mount escapes, devices,
  FIFOs, or sockets unless a narrowly scoped operation explicitly supports
  them;
- revalidate the opened object with file-descriptor metadata;
- enforce entry-count, depth, byte, and expansion-ratio limits on archives;
- never use a web-supplied path as an argument to a shell command; and
- keep staging on the target filesystem when an atomic rename is required.

Storage-pool roots are configured by an administrator and stored with stable
pool IDs plus expected filesystem/mount identity. A missing or unexpectedly
replaced mount fails closed; Helix must not write into an empty mount point on
the root filesystem by accident.

## Critical state database

`helix-state.db` contains users, roles, permissions, session and token
metadata, instance definitions, enabled modules, backup catalog entries,
policies, schedules, port allocations, stable node IDs, audit records,
configuration revisions, and operation/job state.

The implemented founding schema is version 4. Version 1 establishes the
installation identity, local node, migration ledger, and operation ledger.
Version 2 adds constrained owner/user, role, capability, role-assignment,
single-use bootstrap, hashed session/CSRF, security-state, and append-only audit
tables plus authorization-invalidation triggers. Version 3 adds constrained
master-key-version metadata and encrypted secret records, including immutable
identity and monotonic-revision triggers. Version 4 adds the fixed
authentication-audit retention policy, its tracked row count, and the guarded
indexes/triggers used for bounded pruning. Many later domain tables in the
paragraph above remain architectural allocations rather than implemented
features; `PROGRESS.md` is authoritative for that distinction.

Every connection must explicitly set and then query the effective values:

```sql
PRAGMA journal_mode=WAL;
PRAGMA synchronous=FULL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=<bounded measured value>;
```

`journal_mode=WAL` is accepted only if SQLite reports `wal`. Foreign keys are
enabled outside a transaction on every connection and verified; Helix never
depends on a library or compile-time default. Writes are short and use prepared
statements. The initial design is one serialized writer task and a small,
bounded read pool. Synchronous SQLite work runs on a dedicated database worker
or an explicitly bounded blocking executor, not a Tokio core thread.

Each schema object has deliberate `NOT NULL`, `CHECK`, unique, foreign-key, and
index constraints. Stable UUIDs are permanent identities. Human-readable names
and mutable filesystem paths are not keys. Timestamps are stored in a documented
UTC representation. Opaque JSON is allowed only with an owning subsystem and a
schema/version field.

## Recoverable-secret records

The implemented portable store uses XChaCha20-Poly1305 envelope encryption. A
fresh random 32-byte DEK encrypts each 1-to-65,536-byte plaintext, and the active
installation master key encrypts that DEK with a separate fresh 24-byte nonce.
Versioned canonical binary associated data binds the installation, stable
record identity, logical secret identity, data revision, wrap revision, and
master-key identity as appropriate. Moving encrypted material between records
or altering a revision therefore fails authentication.

`master_key_versions` contains only identifiers, lifecycle metadata, and an
authenticated key-check record. `secret_records` contains only metadata,
nonces, ciphertext, and wrapped DEKs. Master-key bytes are carried in a strict
versioned in-memory credential and are absent from SQLite and ordinary online
database backups. The API provides put, metadata, closure-only read,
revision-CAS replace, and revision-CAS delete operations; it has no HTTP or
serialization dependency. The callback scopes the crate-owned zeroizing
plaintext buffer but can explicitly copy bytes, so callers must maintain the
same boundary.

Only initial master-key creation and later credential verification are wired.
The schema reserves lifecycle states, but rotation transitions, crash-safe bulk
DEK rewrapping, systemd credential delivery, fallback key-file handling, and
independent recovery envelopes are not implemented. An ordinary state backup
is intentionally unusable if its separately protected master-key/recovery
material is lost. See [ADR 0005](adr/0005-recoverable-secret-storage.md).

## Metrics database

`helix-metrics.db` is opened through a separate connection manager and failure
boundary:

```sql
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA foreign_keys=ON;
PRAGMA busy_timeout=<bounded measured value>;
```

Samples are batched. High-frequency live samples remain primarily in a bounded
memory ring. Persisted data uses rollups and configurable retention. Metrics
queries and long readers must be bounded so they cannot pin the WAL forever.
If this database is corrupt, Helix moves the main database and every WAL/SHM
sidecar that was present into one private timestamped forensic directory. A
small durable manifest records the exact component set. One deterministic
staging directory is reconciled before metrics opens, so an interrupted move is
completed idempotently on the next startup instead of leaving split evidence.
Ambiguous or unpreservable staging degrades only metrics; critical state remains
available. After the forensic set is published, Helix creates a new metrics
database, raises a health event, and continues serving critical state.

## WAL checkpoint policy

Checkpoint behavior is explicit rather than inherited accidentally:

- the initial state and metrics writer configurations explicitly use a
  1,000-page automatic checkpoint threshold;
- checkpoint results and WAL size are observable so the value can be changed
  from measured evidence;
- read transactions are short; a reader that repeatedly prevents checkpoint
  progress is cancelled or isolated;
- a maintenance task may request a non-blocking `PASSIVE` checkpoint after a
  backup or when the observed WAL exceeds its normal envelope;
- `FULL`, `RESTART`, or `TRUNCATE` checkpoints run only during a quiescent,
  bounded maintenance window, never unpredictably in a latency-sensitive API
  request; and
- low-disk protection treats a growing WAL as critical state, not as disposable
  cache.

The page threshold is an initial policy, not a performance claim. It must be
revisited using database latency, write volume, filesystem behavior, and
power-loss tests.

## Migrations

Migrations are immutable, ordered, and versioned. The application records both
the schema version and the exact migration history it has applied. Startup
refuses a database newer than the running binary understands.

When an existing non-empty critical database is behind the current schema,
Helix creates one verified rollback snapshot of that exact pre-catch-up source,
even when several pending schema steps will be applied:

1. acquire the single migration lock and stop normal writes;
2. run full integrity, foreign-key, schema-history, and semantic validation on
   the source;
3. complete a `TRUNCATE` WAL checkpoint and SHA-256 the resulting durable main
   database file;
4. reconcile deterministic staging names and at most 16 legacy UUID-suffixed
   partials per open; a complete valid partial is published, while an invalid
   partial is removed only after the live source passed validation;
5. look up an alias bound to the source schema version, target schema version,
   and source SHA-256; reuse it only after the aliased content-addressed snapshot
   passes schema, integrity, and hash verification;
6. if no exact alias exists, create one snapshot with SQLite's online backup
   API, normalize its journal, verify it, durably publish it by content SHA-256,
   recheck that the source did not change, and durably publish the alias;
7. apply each pending migration transactionally where SQLite permits;
8. verify the expected schema objects, migration history, integrity, and foreign
   keys, then reopen through the normal path and verify effective PRAGMAs; and
9. retain the rollback snapshot according to the recovery policy.

An identical retry performs no second SQLite backup. A changed source produces
a distinct rollback object. An altered alias or aliased snapshot fails closed
instead of silently creating or trusting a replacement.

If any step fails, Helix does not mark the migration complete and does not
silently delete the backup. A migration involving non-transactional filesystem
work uses the operation ledger and staged, idempotent steps. CI must exercise
every supported released schema directly to the current schema; testing only a
fresh database is insufficient.

Copying only a live WAL-mode `.db` file is not a valid snapshot. The WAL is part
of live database state. The migration source is explicitly checkpointed before
identity calculation, and Helix uses SQLite's supported online backup mechanism
for the consistent rollback object.

## Crash-safe Helix-owned file writes

Critical configuration uses one shared primitive with the following contract:

1. validate and size-limit the requested content;
2. open the trusted parent directory and existing target without following
   links;
3. read and retain the current revision;
4. create a random, exclusive temporary file in the same directory;
5. write the complete content and validate the staged file;
6. preserve the required mode and ownership without broadening access;
7. flush application buffers and `fsync` the temporary file;
8. atomically rename within the same filesystem;
9. `fsync` the parent directory;
10. reopen and parse the published file; and
11. commit the configuration revision and operation completion record.

SQLite durability is delegated to SQLite and its VFS; Helix does not manually
copy, rename, or fsync files behind an open database connection.

Network filesystems, USB bridges, RAID controllers, and drives may not honor
locking or flush requests correctly. The critical databases default to local
storage. A non-local or unusual filesystem requires an explicit compatibility
decision and cannot inherit the same durability claim without fault testing.

## Cross-database and filesystem operation ledger

SQLite and the filesystem do not share one ACID transaction. Every important
multi-step operation records:

- operation UUID and type;
- initiating actor and authorization decision;
- stable target IDs, never display-name paths;
- desired state and previous revision;
- staged resource locations and expected hashes;
- current state, attempt count, timestamps, and last safe checkpoint;
- cancellation semantics; and
- one of `resume`, `roll_back`, `restore_previous`, `quarantine`, or
  `needs_operator` as the recovery disposition.

Steps are idempotent or have an explicit compensating action. On startup Helix
reconciles unfinished operations before accepting a conflicting mutation.
Unknown half-applied state is surfaced; it is never converted into success.

## Low-disk and write-protect behavior

Every storage pool has configurable warning, critical, and emergency headroom.
Checks use both free bytes and free percentage, account for the estimated
staging plus final size, and are repeated before the destructive/publish step.
Sparse files, snapshots, quotas, reserved blocks, and concurrent writers mean
the estimate is not a guarantee.

At warning level Helix alerts and defers optional cache growth. At critical
level it pauses metrics persistence, cache fills, archive creation, optional
compression, and new installs. At emergency level it rejects all non-recovery
high-write operations, keeps the API available in a reduced read/recovery mode,
and preserves room for audit and database recovery. It does not automatically
delete worlds, the newest valid backup, forensic copies, or unknown user files.

Retention and cache eviction are bounded jobs with leases. A file referenced by
an active install, backup, restore, or import cannot be evicted. If cleanup
cannot restore the margin, Helix asks for operator action rather than claiming
the operation completed.

## Integrity and shutdown checks

Routine health includes bounded `quick_check` and `foreign_key_check`; a full
`integrity_check` runs periodically or during an explicit maintenance window.
The state database records whether the prior shutdown completed cleanly. After
an unclean shutdown Helix performs additional checks before accepting writes.

Before any destructive repair Helix makes a byte-preserving forensic copy of
all database components and records tool/version/error context. State repair is
an explicit operator workflow. Metrics recreation is automatic only after the
forensic copy succeeds or the operator knowingly waives it because the medium
cannot be written.

## Verification gates

This design is not complete until Linux integration tests demonstrate:

- every connection's effective PRAGMAs;
- migration backup, rollback, and upgrades from every supported schema;
- live online backup during concurrent writes;
- process kill and power-loss injection around database and config writes;
- disk-full, read-only filesystem, permission failure, and WAL growth;
- path traversal, symlink races, mount replacement, and malicious archives;
- corrupt-state fail-closed behavior and metrics-only degradation;
- operation reconciliation after each durable checkpoint; and
- verified restore with the old state retained for rollback.

## Current external assumptions

- SQLite WAL and synchronous behavior follows the current official SQLite
  documentation: <https://www.sqlite.org/wal.html> and
  <https://www.sqlite.org/pragma.html>.
- Live database snapshots use the official online backup interface:
  <https://www.sqlite.org/backup.html>.
- Linux constrained path resolution is based on `openat2(2)` semantics:
  <https://man7.org/linux/man-pages/man2/openat2.2.html>.

These assumptions must be revalidated when the implementation pins its SQLite
and minimum Linux versions.
