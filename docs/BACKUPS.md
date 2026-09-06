# Vault Backup and Restore Design

## Status

Vault is a design only. The foundation implements one narrower primitive:
`helixctl backup-state` uses SQLite's online backup interface, validates the
resulting state database, and publishes a database-only snapshot without
overwriting an existing destination. Temporary-link cleanup is retried; a
persistent cleanup failure is returned as a successful publication with the
exact removable residue path, not as a failed backup. That command is not a
Vault backup, game-data backup, retention policy, or restore proof.

There is no implemented Vault backend, scheduler, encryption format,
application-consistency hook, retention engine, restore, or repository
verification job. The words `Verified` and `Test Restored` are reserved Vault
states and must never be displayed for mocked or partial work.

Vault protects Helix critical state and selected game/service data. Database
snapshots complement game-data backups; neither replaces the other.

## Goals and non-goals

Vault must provide:

- manual, scheduled, pre-update, pre-configuration, and pre-game-upgrade backups;
- local and eventually off-host destinations behind a typed backend interface;
- application-consistent capture where a tested game mechanism exists;
- authenticated integrity metadata and optional/required encryption by policy;
- bounded retention, cancellation, progress, and low-disk preflight;
- staged restore, rollback, and periodic test restore; and
- enough self-contained metadata to recover when `helix-state.db` is gone.

An archive opening successfully is not integrity verification. A checksum pass
is not an application-consistency test. A restore command existing is not proof
that fresh-machine recovery works.

## Backup domains

A Vault backup can include independently declared datasets:

- a consistent `helix-state.db` online snapshot;
- selected instance server/data/config/metadata trees;
- configuration revision payloads and operation metadata needed to reconcile;
- Sequence and Strand package references plus verified content when requested;
- Lattice, policy, schedules, and other critical control-plane data;
- selected system configuration that Helix owns or has explicitly adopted; and
- wrapped key material required for authorized disaster recovery.

Metrics, caches, temporary files, active sockets, transient worker state, and
ordinary console buffers are excluded by default. Raw installation master keys,
session bearer tokens, plaintext passwords, and the recovery secret are never
included.

## Backend boundary

All repositories implement a typed `BackupBackend` contract for:

- capability and health discovery;
- begin/write/finalize/abort snapshot;
- bounded streaming reads and writes;
- integrity verification;
- atomic or transactional publication semantics;
- listing by authenticated manifest;
- retention deletion with explicit object ownership; and
- restore into a caller-provided staging root.

The first backend should be a simple local repository with clear filesystem
semantics. A future Restic backend runs as a one-shot worker; Restic is not a
resident Helix dependency. Its binary source, version, hash/signature, command
arguments, environment, repository target, and exit status are verified and
redacted. Helix does not parse free-form user options into a privileged Restic
command.

Backend success is accepted only after Helix can read back and authenticate the
published manifest. A backend's provider-specific "uploaded" result alone is
not a complete backup.

## Backup state machine

Catalog state is explicit:

```text
Planned -> Preparing -> Capturing -> Finalizing -> Created
                                   \-> Failed
                                   \-> Cancelled

Created -> Verifying -> Verified | VerificationFailed
Verified -> TestRestoring -> TestRestored | TestRestoreFailed
```

State transitions are append-audited and monotonic except for a new verification
attempt. `Created`, `Verified`, and `TestRestored` remain distinct. Failure and
cancellation never publish an incomplete snapshot under a successful ID.

Each job has a cancellation contract. Cancellation is checked between safe
chunks and never interrupts the atomic publication step. Cleanup is idempotent
and represented in the operation ledger if it cannot finish immediately.

## Manifest

Every backup has a unique random ID and a versioned, size-limited,
authenticated manifest containing at least:

- backup and repository IDs, creation start/end, Helix version, local node ID;
- manifest/schema/Genome/Sequence versions relevant to restore;
- actor and policy/schedule ID without bearer credentials;
- dataset roots by stable ID and declared logical type;
- consistency method and whether each hook completed;
- entry count, logical/stored bytes, compression and encryption metadata;
- for every required object: normalized logical path, type, size, cryptographic
  digest, and storage-object reference;
- state DB schema version and integrity-check result;
- source filesystem/snapshot information and important warnings;
- completion, verification, and test-restore states as separately authenticated
  records; and
- wrapped recovery-key material only when the configured recovery design calls
  for it.

The catalog in `helix-state.db` is an index, not the sole source of backup
existence. A fresh Helix installation can enumerate a repository from its
authenticated manifests. Unknown fields follow version compatibility rules;
unknown required semantics cause a safe rejection.

Cryptographic hashes such as BLAKE3 are suitable for fast internal content
integrity; SHA-256 may be retained for ecosystem compatibility. Non-
cryptographic checksums are not proof of integrity. The chosen algorithm and
digest encoding are explicit and versioned.

## Critical database snapshot

Helix snapshots a live `helix-state.db` through SQLite's online backup API. It
does not copy only the main file of an open WAL database. The destination is a
unique staging file, opened and written by SQLite. After completion Helix closes
and reopens it, checks the expected schema version, `integrity_check`, and
`foreign_key_check`, flushes and durably publishes it, then includes its digest
and result in the manifest.

An online snapshot is logically consistent at SQLite's boundary. It does not
by itself make external game files consistent with catalog state. Operations
that span the database and filesystem use a ledger checkpoint and explicit
capture order.

State snapshots are small and frequent. Initial retention policy should expose
configurable recent-hourly, daily, and weekly tiers, but exact defaults require
measured storage growth and product review. Migration backups are separately
protected from routine retention until the matching application rollback window
expires.

## Game/application consistency

Each Sequence declares one reviewed consistency strategy:

- filesystem snapshot while the game is quiescent;
- tested save/flush protocol with guaranteed resume cleanup;
- bounded pause;
- orderly stop and later restoration of the prior running state; or
- explicit crash-consistent capture with a warning when no stronger mechanism
  is known.

For a Minecraft mechanism such as `save-off`, `save-all flush`, capture, and
`save-on`, every command, acknowledgement, timeout, server version, and failure
cleanup must be tested before Helix calls the backup hot/application-consistent.
If the flush cannot be confirmed, the job either follows a policy-approved stop
path or fails honestly.

The workflow records whether a service was running. It restarts only services
that were running before capture unless the operator explicitly asks otherwise.
A failed or cancelled backup must execute the resume/save-enable cleanup even
when copying or the backend fails.

Btrfs, ZFS, and reflink backends are optional `SnapshotBackend` implementations.
Detection is not permission to use one. Helix verifies mount/dataset identity,
space behavior, readonly semantics, cleanup, and application quiescence. On
ordinary local filesystems it uses safe staged conventional copying.

## Streaming and resource bounds

Backup, verification, and restore stream data; they do not read whole archives
or worlds into memory. Workers have explicit CPU weight, memory maximum, I/O
priority, open-file limit, temporary-space quota, progress cadence, timeout, and
cancellation behavior. Concurrency is bounded per disk and destination so a
backup does not starve a running game or `helix-state.db`.

File trees have entry, depth, individual-file, total logical-byte, and sparse-
extent limits. Source changes during a conventional copy are detected where
possible and reported as an inconsistent capture, not ignored.

## Space preflight and failure

Before capture, Helix estimates source bytes, staging overhead, compression
uncertainty, repository growth, rollback requirements, and emergency headroom
for every affected pool. It rechecks during streaming and before publication.
Remote free-space or quota information is treated as advisory when the provider
cannot guarantee it.

If a disk fills, Helix stops optional high-volume work, aborts the incomplete
snapshot safely, restores application save state, preserves previous valid
backups, and records cleanup work. Retention never runs first as an unbounded
attempt to make a failing backup fit and never deletes the last known-good copy.

## Encryption and recovery keys

Off-host and removable-media policies default to authenticated encryption.
Full-clone/secret-bearing backups require it. Encryption uses a reviewed,
versioned format and library with random data keys and nonces. Per-backup data
keys are wrapped rather than re-encrypting all content during installation-key
rotation.

The raw installation master key and recovery secret are not stored in the Vault.
The installation may store key material wrapped by an independent, user-held
recovery secret. Password-based wrapping uses a reviewed memory-hard KDF with
parameters recorded in the envelope and bounded during parsing. Parameters are
calibrated and upgradeable; no custom cryptography is introduced.

Manifest fields required to locate and decrypt content are authenticated. If
some repository metadata remains visible, the UI documents exactly what an
observer can learn. Losing both the installation key and recovery secret may
make encrypted secrets unrecoverable.

## Verification

`Created` requires durable backend publication and manifest readback.
`Verified` additionally requires:

1. manifest authentication and supported version;
2. existence, length, and cryptographic digest of every required object;
3. decryption/authentication where encrypted;
4. state database schema and integrity checks;
5. no missing consistency completion marker; and
6. backend-specific repository checks.

Verification samples are insufficient for a `Verified` label unless the UI
explicitly labels them as sampled. Full verification is bounded and may run as
a one-shot worker. Failures quarantine the affected backup in the catalog and
do not delete evidence automatically.

`Test Restored` requires restoration into an isolated temporary root, full
manifest/object verification, database and structure validation, game/config-
specific checks, cleanup, and a recorded test environment. Starting an actual
game may be a deeper certification tier; if omitted, the UI says so.

## Restore

Restore never writes directly into a live target tree. It authenticates the
manifest, resolves all objects, preflights staging plus rollback space, extracts
with hostile-archive rules, restores safe policy ownership/modes rather than
untrusted numeric owners, and validates before publish. The existing tree is
retained as a rollback revision until post-restore health succeeds.

Cross-filesystem publish cannot rely on rename. It uses a copy plus operation
ledger with an explicit cutover point and enough retained data to recover after
each crash. On the same filesystem, directory exchange/rename is used only
after all open-handle and service-state assumptions are tested.

A wrong key, unsupported format, missing object, digest mismatch, catalog/
manifest disagreement, path escape, or insufficient space rejects the restore
before touching the live target.

## Retention and immutability

Retention is policy-driven and evaluated only over successfully published
backups. It protects the newest valid copy, the newest verified copy, required
hourly/daily/weekly tiers, legal/operator holds, pre-migration rollback points,
and in-progress restore sources. Deletion produces a preview and audit record,
then uses backend-specific idempotent deletion.

Vault supports immutable/object-lock repositories where a backend provides
them, but does not claim immutability for ordinary writable local directories.
At least one off-host or offline copy is required for a disaster-ready label.
A backup on the same SSD does not protect against that SSD's loss.

## Restic integration constraints

Before a Restic backend is enabled, Helix must pin a supported version range,
verify downloaded packages against upstream evidence, protect repository
credentials, use a dedicated worker/service sandbox, bound output, parse stable
machine-readable results where available, and test repository locking,
interruption, prune/check, wrong-password, partial network, and restore.

Helix never calls a Restic invocation `Verified` merely because the process
returned zero. Its own expected manifest and restore checks remain authoritative
for the Helix backup contract.

## Required tests

The Vault release gate requires:

- live SQLite online snapshot during concurrent writes;
- consistency-hook success, timeout, server crash, and cleanup failure;
- kill/cancel/network loss/disk full at every state transition;
- corrupt manifest, object, ciphertext, repository index, and state snapshot;
- symlink, hard-link, device, path traversal, archive bomb, and sparse-file
  attacks during restore;
- retention with holds, last-good protection, and interrupted deletion;
- wrong key, lost original machine, and fresh-machine recovery;
- same- and cross-filesystem rollback after publish failure;
- test restore with recorded data hashes and application-specific validation;
  and
- proof that metrics/backend failure does not block local critical recovery.

Official SQLite backup assumptions come from
<https://www.sqlite.org/backup.html> and WAL behavior from
<https://www.sqlite.org/wal.html>. The implementation must revalidate these
against its pinned SQLite library and supported filesystems.
