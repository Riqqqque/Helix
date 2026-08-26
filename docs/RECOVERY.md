# Recovery Model and Runbooks

## Status

This document defines intended recovery behavior. The foundation currently has
`helixctl doctor`, a database-only `backup-state` command, state/metrics
integrity checks, an unclean-shutdown check, and metrics corruption
preservation/recreation. Those are useful primitives, not a complete recovery
system. They have portable unit/integration coverage plus one Windows
release-binary forced-stop recovery and clean Ctrl+C run. One scoped Ubuntu 24.04
package lifecycle also passed forced-crash systemd restart, clean stop/start,
full doctor, verified database-only backup, explicit package-file rollback, and
data-preserving uninstall for commit `6868b36`. Complete Linux fault, database
restore, disk-full, power-loss, and fresh-machine recovery evidence is still
absent.

State-database recovery, migration rollback across a released production schema,
operation reconciliation, Vault restore, disk-full handling, and fresh-machine
restore are not implemented or proven. Other command names below are interface
targets, not usable instructions. A runbook becomes operational only after its
command, tests, and supported-version evidence are linked from `PROGRESS.md`.

Helix must preserve evidence and the last known-good copy before attempting a
repair. It never silently deletes a corrupt database, overwrites a failed
instance, or calls a readable archive a verified restore.

## Recovery priorities

When failure occurs, the order is:

1. protect people and the host from unsafe automated actions;
2. stop new conflicting writes while leaving independent game services alone
   unless their consistency requires intervention;
3. preserve original data, WAL/journal files, error context, and operation state;
4. identify the authoritative source and exact failure boundary;
5. stage and verify recovery away from the original;
6. publish with an atomic or explicitly journaled transition;
7. validate runtime behavior, not only file readability; and
8. retain a documented rollback until the operator accepts the recovery.

Metrics, caches, and optional integrations may degrade so that critical state
and local recovery remain usable.

## Recovery evidence

Every recovery attempt receives an operation UUID and records:

- initiating actor, authorization, Helix version, schema/format versions, and
  host identity;
- failure classification and original error without secret material;
- affected stable IDs and storage pools;
- source and destination paths as internal handles, not user display names;
- hashes/manifests and checks performed;
- each durable checkpoint and its result;
- files retained for rollback or forensic analysis; and
- final disposition: recovered, rolled back, quarantined, or operator action
  required.

The record is useful evidence, not tamper-proof against root.

## Startup after an unclean shutdown

The state database contains a clean-shutdown marker. Startup reads the previous
value and completes any required full integrity validation before bounded
session cleanup, then sets the current run to unclean in a committed
transaction. Shutdown first drains HTTP work, waits for every tracked detached
state/password blocking task to finish, and only then marks the run clean. The
implemented drains share a 20-second deadline beneath systemd's 30-second stop
timeout. A deadline expiry force-closes the process without writing the clean
marker.

`helixctl backup-state` verifies and durably publishes the destination before
removing its temporary hard link. Cleanup is retried three times. If cleanup
still fails, the command reports a successful snapshot plus the exact
`.partial-*` residue path; that verified residue may be removed after the
published destination is confirmed readable.

The implemented foundation performs the full state integrity check before
session cleanup or normal service. The complete target recovery flow, before
accepting mutations, additionally requires Helix to:

1. verify database files and parents have expected type, ownership, and mode;
2. open the critical database without discarding its WAL;
3. verify effective PRAGMAs, schema version, `quick_check`, and
   `foreign_key_check`;
4. inspect all in-progress operations and jobs;
5. verify staged files by stable operation ID and expected hash;
6. resume, roll back, quarantine, or block each operation according to its
   recorded recovery disposition; and
7. report reduced mode until required reconciliation is complete.

Only the database validation ordering is implemented today; operation/job and
staged-file reconciliation in the list above remain target behavior. Metrics
recovery happens independently. No startup path may recreate the state database
merely because opening it failed.

## State database corruption

Symptoms include SQLite corruption codes, failed integrity checks, an invalid
schema/migration history, or impossible constraints. The intended
`helixctl doctor` and `helixctl state recover` flow is:

1. stop `helixd` writers while leaving unrelated systemd-managed games in their
   current state;
2. capture the database, `-wal`, `-shm`, file metadata, filesystem/mount
   information, Helix/SQLite versions, and relevant redacted logs into a
   timestamped read-only forensic directory;
3. inventory verified online snapshots and Vault copies without modifying them;
4. stage the newest compatible candidate in a separate directory;
5. open it with the target SQLite library and run schema-version,
   `integrity_check`, and `foreign_key_check` validation;
6. compare important counts and stable IDs with the backup manifest and inspect
   unfinished operations;
7. retain the damaged original, atomically publish the staged candidate, sync
   the parent directory, and reopen it through the production connection path;
8. reconcile filesystem state against the restored catalog; and
9. enter recovery review instead of automatically starting destructive jobs.

SQLite salvage/recovery tools may produce a best-effort candidate only after the
forensic copy. Salvaged data is never installed in place and never labeled
verified solely because it parses. If no verified candidate exists, Helix fails
closed and asks for explicit operator direction.

## Metrics corruption

Metrics are non-authoritative. When `helix-metrics.db` is corrupt, Helix:

1. stops metrics writers/readers;
2. moves the database and the exact WAL/SHM set that was present into one
   private timestamped forensic directory with a durable manifest;
3. creates and migrates a new metrics database at a staged path;
4. verifies it, atomically publishes it, and restarts collection at a low rate;
5. raises a persistent health notification describing the lost interval; and
6. keeps authentication, instance control, files, backup, and recovery usable.

Failure to preserve a copy because the disk is full is reported explicitly. It
does not justify deleting the critical state database.

Forensic publication uses one known staging directory. Startup reconciles that
directory before opening metrics: components already staged are retained,
components still at the source are moved, and the complete directory is
published atomically. A crash before the manifest is published is safely
retried when no component moved. Missing, duplicated, or unexpected components
fail the metrics recovery closed without blocking the critical-state domain.

## Failed migration or downgrade

Every catch-up attempt from an existing non-empty schema has one verified online
snapshot of the exact pre-catch-up source. Its alias binds source schema, target
schema, source SHA-256, and content-addressed snapshot SHA-256. An identical
retry revalidates and reuses that snapshot without another backup; a changed
source creates a distinct rollback object; an altered alias or snapshot fails
closed. Startup reconciles at most 16 legacy partials per open so hostile or
accidental staging debris cannot create unbounded work.

On migration failure, Helix keeps normal writes disabled and either leaves the
transactionally unchanged source in place or restores the verified snapshot to
a staged path. It never attempts to continue with an unknown partial schema.

The normal binary refuses a schema newer than it supports. Downgrade is not an
implicit reverse migration. It requires a documented compatible path, commonly
restoring the pre-upgrade database and matching application version. Filesystem
side effects use the operation ledger and must be individually reversible or
explicitly require operator action.

CI must test direct upgrades from each supported released schema, injected
failure at every migration boundary, a killed process, a full disk, and the
matching application rollback.

## Interrupted critical file operation

An atomic rename protects one publication step, not the entire database-plus-
filesystem workflow. At restart Helix uses the operation ledger to compare:

- database revision and operation state;
- current target and staged file existence, type, metadata, and hash;
- previous revision and rollback payload; and
- any external service state changed by the operation.

It then performs the recorded idempotent next step, restores the previous
revision, quarantines unexplained data, or requests intervention. A missing
temporary file is not automatically success. An existing final file is not
automatically owned by Helix unless its stable ID and expected digest match.

## Low disk and disk full

At low-space warning Helix suspends optional cache fills and alerts. At critical
space it pauses metrics persistence, compression, archive generation, downloads,
new installs, and other high-volume work. At emergency space it serves reduced
read/recovery APIs and rejects non-recovery mutations.

Recovery order is conservative:

1. stop optional writers and bound logging;
2. report the pool, filesystem, free bytes/percentage, quota, reserved margin,
   WAL and staging usage;
3. evict only reproducible cache objects with no active lease;
4. apply already-approved bounded log retention;
5. offer operator-selected cleanup candidates;
6. checkpoint only if it can be done safely and actually helps; and
7. re-run integrity checks before returning to normal write mode.

Helix never automatically deletes worlds, user files, the newest known-good
backup, forensic evidence, or an in-progress restore source. A failed write is
not retried in a tight loop.

## Failed configuration change

For Helix-owned configuration, the previous parsed revision and metadata are
retained before publication. If the new file fails post-write parsing or the
managed service rejects it, Helix stages the previous revision, republishes it
through the same fsync/rename primitive, verifies it, and records the rollback.
If the service was running before the operation, Helix restores only that prior
state; it does not start an intentionally stopped service.

Externally managed files discovered through adoption are backed up and reviewed
before Helix assumes ownership. A conflict from an external editor is presented
as a three-way decision; Helix does not overwrite it based only on timestamp.

## Failed backup

A backup remains `Creating` until its application-consistency hooks, data copy,
manifest, cryptographic integrity metadata, catalog publication, and durable
backend commit all succeed. On cancellation or failure:

- game save state is restored in a guaranteed cleanup path;
- incomplete repository objects are left for the backend's safe cleanup, not
  published as a backup;
- the previous valid backup and retention set remain untouched; and
- any cleanup failure becomes a reconciled operation rather than a hidden temp
  directory.

See `docs/BACKUPS.md` for the full state model.

## Restore of an instance

Restore is a staged, rollback-protected operation:

1. authorize `backups.restore` for the exact instance and backup;
2. verify backup status, format/schema compatibility, encryption keys,
   manifest authentication, and every required object/hash;
3. preflight enough space for staging, rollback, and emergency margin;
4. record whether the instance was running;
5. run the Sequence's tested consistency/stop hook and wait for systemd to
   confirm the desired state;
6. restore into a new UUID-named staging directory without following archive
   links or trusting stored ownership;
7. validate structure and game/config-specific invariants;
8. retain the current data as a rollback revision and atomically exchange or
   ledger the publish transition;
9. restore safe ownership/modes from policy, not attacker-controlled archive
   numeric IDs;
10. start only if it was running before or the operator explicitly requested
    start, then perform lifecycle health checks; and
11. roll back automatically if validation/start fails and rollback remains safe.

The UI distinguishes `Created`, `Verified`, and `Test Restored`. Only the last
state demonstrates that a restore workflow succeeded under the recorded test
conditions.

## Full Helix state restore

Full state restore requires maintenance mode because authentication,
authorization, and job state are being replaced. The candidate is restored
away from live paths, its manifest and database are verified, secrets are
unwrapped with the installation or recovery key, and compatibility migrations
run only after an additional snapshot.

The workflow inventories current host hardware, storage pools, ports, network
interfaces, installed runtimes, and service units. Differences are shown before
publication. Port, mount, user, and node-ID conflicts require a deterministic
remap plan. Services are not started until definitions, filesystem data, keys,
units, and policy references agree.

Restoring older users and permissions can revoke the current session or restore
historical credentials. The operator must reauthenticate with recovery authority
and receives a preview of that effect.

## Fresh-machine disaster recovery

A fresh machine must be recoverable without the dead host. The target flow is:

1. install a compatible, verified Helix package on supported Ubuntu;
2. choose recovery mode before creating a conflicting owner;
3. connect to a Vault using locally entered credentials;
4. supply the independent recovery secret to unwrap required key material;
5. list only manifests whose authenticity and format can be verified;
6. stage the selected Helix state and run database integrity checks;
7. compare CPU, memory, architecture, filesystems, mounts, ports, and runtimes;
8. approve necessary remaps and unavailable optional features;
9. restore instance data with per-game validation;
10. create independent systemd units with least privilege;
11. start only approved workloads and run health checks; and
12. retain a signed or hashed recovery report and rollback data.

Losing both the original installation key and its independent recovery secret
may make encrypted credentials unrecoverable. World data may still be
recoverable if it was backed up without that key, but Helix must not imply full
recovery.

## Compromised host or key

If root, the installation key, or a release signing key may be compromised,
ordinary in-place recovery is not trusted. Use a clean machine and verified
offline installer, rotate trust roots and all potentially exposed credentials,
restore data from a point before compromise, review Strands/Sequences/plugins,
and compare off-host audit/backup evidence. Helix cannot prove that a root-
controlled local forensic copy is truthful.

## Required recovery drills

No recovery claim is publishable until automated tests and clean Ubuntu drills
cover:

- process kill and simulated power loss during config write, DB commit,
  migration, backup, restore, import, and publish;
- corrupt state DB, corrupt metrics DB, missing WAL, read-only media, invalid
  permissions, and replaced mount;
- disk full before staging, during copy, before rename, and during rollback;
- migration upgrade and application rollback from every supported release;
- corrupted/truncated/tampered backup and wrong recovery key rejection;
- full instance restore and fresh-machine recovery with port/storage remapping;
- recovery when remote Vault/DNS/Internet is unavailable; and
- proof that game services running before a panel restart remain running and
  intentionally stopped services remain stopped.

Each drill records hardware/filesystem, Helix and dependency versions, injected
fault, expected invariant, observed result, data hashes, duration, and cleanup.

## Root and hardware limits

Recovery depends on the storage stack honoring writes, flushes, locks, and
atomic rename semantics. Helix cannot repair undetected malicious firmware,
guarantee truth after unrestricted root compromise, recover encryption without
any surviving key material, or promise application consistency for a game whose
save protocol is unknown or compromised. These limits must appear in product
copy wherever the stronger interpretation would be plausible.
