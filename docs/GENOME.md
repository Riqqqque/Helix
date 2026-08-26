# Genome Portability Format

## Status

Genome is a founding design, not an implemented file format. No exporter,
importer, encryption envelope, parser, compatibility migration, or recovery test
exists yet. Helix must not emit a file named `.helix-genome` until a format ADR
pins the byte-level container and the security/failure tests in this document
pass.

This document defines the logical version-1 contract so implementation choices
cannot accidentally weaken portability or recovery. The physical container,
canonical serialization, compression, and cryptographic library remain open
decisions that require an ADR, benchmarks, test vectors, and external review.

## Purpose

A Genome is a portable, self-describing representation of selected
Helix-managed configuration and, optionally, data. It is not automatically a
backup: an export can omit history, secrets, or bulk data, and it has no
retention or independent restore evidence unless a Vault policy supplies those
properties.

Genome supports two modes:

- **Blueprint:** configuration, definitions, policies, layouts, package
  references, and selected users without bulk worlds/saves by default.
- **Full Clone:** Blueprint plus selected instance files, worlds/saves,
  mods/plugins, locally required packages, and optional encrypted secrets.

An export selection is explicit. "Include users" and "include secrets" are
separate decisions. Session tokens and one-time recovery material are never
exported.

## Logical container

The logical format contains:

```text
header
manifest
records/
objects/
integrity/
optional-key-envelope
```

The header provides a magic value, container format version, manifest location
and size, required feature flags, and encryption/KDF metadata needed before the
manifest can be opened. Header and manifest parsing are strictly length-bounded.

The manifest is a versioned typed document. It includes:

- Genome UUID, creation time, creator Helix version, mode, and source node ID;
- manifest/schema versions and minimum compatible Helix version;
- selected datasets and explicit exclusions;
- stable IDs, logical relationships, and object ownership versions;
- source architecture, CPU/memory facts relevant to compatibility, storage-pool
  characteristics, runtime requirements, and port assignments;
- Sequence/Strand identifiers, versions, origin, integrity/provenance evidence,
  required capabilities, and whether package bytes are embedded;
- each object type, logical path, exact stored/logical size, digest algorithm
  and digest, compression/encryption metadata, and required/optional flag;
- state database export schema and migration requirements;
- secret inclusion/encryption declaration without plaintext values;
- warnings about unsupported or crash-consistent-only datasets; and
- authenticated completion metadata.

Records represent typed control-plane objects rather than an opaque dump of the
live SQLite file. A Full Clone may additionally carry a verified SQLite snapshot
for disaster fidelity, but import still validates and migrates through explicit
schema rules. Opaque subsystem JSON requires an owner and version.

Objects are content-addressed, streamed, and referenced by digest. The same
payload need not be stored twice. The implementation must define whether object
names expose their digest when encryption is used and document any metadata
leakage.

## Stable identity and portability

UUIDs identify nodes, instances, users, dashboards, policies, storage pools,
operations, and packages. Display names, source hostnames, absolute filesystem
paths, Linux numeric UIDs/GIDs, and interface names are not portable identity.

The source local `NodeId` is recorded for provenance. Exact Clone may preserve
it only when it cannot collide with an existing node. Portable Clone creates a
new local node and records an import mapping. Every rewritten reference is
included in the preview and durable import report.

Filesystem objects use normalized logical paths relative to a typed dataset
root. No object contains a trusted destination absolute path. Archive ownership
is a logical role; target UID, GID, mode, ACL, capabilities, and service identity
are derived from target policy.

## Export policy and privacy

Before export, Helix displays a structured inventory of included and excluded
data, estimated logical/stored size, secret-bearing fields, personal data,
external package references, and portability warnings. The defaults exclude:

- recoverable secrets and raw key material;
- password-equivalent session/API/recovery tokens;
- metrics, cache, temporary files, console buffers, and support bundles;
- unselected user files; and
- host-specific credentials or system configuration that cannot be safely
  remapped.

Passwords remain Argon2id hashes if users are included. Their inclusion still
permits offline guessing and therefore makes the export sensitive. MFA recovery
codes, if supported, are rotated or excluded rather than silently cloned.

Blueprint exports may be unencrypted only after an explicit privacy review and
clear warning. Full Clone and any export containing credentials, password
hashes, private worlds, or personal data require authenticated encryption.

Helix writes to a unique same-filesystem temporary path, streams and hashes all
records/objects, authenticates the final manifest, flushes the file, atomically
renames it, and fsyncs the parent directory. Cancellation never publishes a
partial Genome under the final name.

## Encryption and recovery

Genome encryption uses a reviewed versioned envelope, never custom
cryptography. The target design is:

- a random per-export data-encryption key;
- authenticated streaming/chunk encryption with unique nonces and chunk order
  bound into associated data;
- header/manifest authentication so selection and compatibility metadata cannot
  be rewritten;
- one or more recipient envelopes, such as a user passphrase or explicit target
  key; and
- recorded algorithm, KDF, salt, parameters, key version, and test vector.

Passphrase wrapping uses a reviewed memory-hard KDF with calibrated,
upgradeable parameters. Import enforces upper bounds before allocating memory
or CPU so attacker-chosen KDF settings cannot exhaust the server. Export refuses
an empty or policy-inadequate password and offers a generated recovery secret.
The passphrase/recovery secret is not embedded in or logged beside the Genome.

An installation master key is included only as material wrapped by an
independent export recipient. Raw local keys and systemd credential files are
never copied. If secrets are excluded, imported integrations remain visibly
unconfigured instead of containing placeholders that appear valid.

## Import is hostile input

Import parsing runs in a one-shot worker with bounded memory, CPU, I/O, file
descriptors, wall time, extracted bytes, item count, nesting depth, and
compression ratio. No executable content runs during inspection.

The parser rejects:

- unsupported required features or versions;
- absolute paths, `..`, duplicate/conflicting normalized paths, NULs, and
  platform-invalid names;
- symlinks, hard links, devices, FIFOs, sockets, setuid/setgid bits, file
  capabilities, and attacker-controlled ownership;
- entries exceeding declared or policy size/count/depth limits;
- truncated, duplicate, reordered, missing, or digest-mismatched chunks;
- manifest/object disagreement and unauthenticated metadata;
- unknown required record types or migrations; and
- nested Genome/archive recursion unless a reviewed format version explicitly
  permits it with a lower bound.

Parsing, inspection, and decryption never write into `/var/lib/helix/instances`
or the live state database. They produce a typed staged inventory under an
operation UUID.

## Inspection and dry-run plan

Before mutation, Helix presents:

- verified origin/integrity evidence and whether authenticity is known;
- included users, roles, instances, worlds, packages, policies, dashboards, and
  secrets;
- source/target version and unsupported-feature findings;
- storage required for staging, final data, rollback, and emergency margin;
- target CPU architecture, memory, runtime, filesystem, and service differences;
- every ID, storage pool, path, port, node, username, and package remap;
- permissions requested by imported Sequences/Strands;
- data that will be skipped or cannot be validated; and
- services that would be created, changed, stopped, or started.

Inspection is safe and repeatable. It may contact package catalogs only after an
explicit option and the same SSRF/download policy as normal installs. It does
not download or execute missing packages merely to render a preview.

## Conflict handling

### Exact Clone

Exact Clone is intended for an empty recovery target. It fails on conflicting
stable IDs, existing owner state, occupied ports, incompatible architecture, or
non-empty destination roots unless the recovery workflow explicitly proves the
existing data is the rollback copy being replaced. It never silently merges
security principals.

### Portable Clone

Portable Clone may generate new target IDs and offer deterministic remaps. Port,
storage, username, and package conflicts are resolved in a versioned import
plan. The operator must approve destructive or security-relevant changes.
Unresolved references remain disabled with an actionable error; they are not
dropped silently.

User merge requires proof of intended identity. A matching display name or
email is insufficient. Role/capability escalation is shown separately and
requires `users.manage` plus recent reauthentication.

## Transaction and crash recovery

Genome import is an operation-ledger workflow:

1. authenticate/decrypt and inspect without live mutation;
2. record the approved, hashed import plan;
3. reserve IDs, ports, storage, and required capacity transactionally;
4. stage all filesystem objects under target-pool operation roots;
5. verify hashes, parse configs, and prepare systemd definitions offline;
6. make a verified state database snapshot and per-target rollback revision;
7. publish filesystem trees by atomic same-filesystem transitions or explicit
   cross-filesystem ledger checkpoints;
8. commit state objects and remap references in short transactions;
9. create/enable services through typed privileged operations;
10. validate without automatically starting services unless approved; and
11. mark complete only after a restart reconciliation dry run succeeds.

Each step is idempotent or has a compensating action. After a crash, Helix can
resume, restore previous data, quarantine staged content, or ask for operator
action. It never infers success from a partially populated destination.

## Package and executable trust

Embedding a Sequence, Strand, game binary, mod, or plugin preserves bytes; it
does not make them safe. The manifest records origin, version, digest,
signature/provenance status, requested capabilities, certification status at
export time, and whether Helix can currently revalidate it.

Import never executes embedded content during validation. Permission grants do
not transfer silently to a different Helix version or host. Changed or unknown
capabilities require review. Native sidecars remain high-trust code even when a
Genome is authentic.

## Versioning

Container, manifest, record, secret-envelope, and owned configuration schemas
are independently versioned. Helix declares a read range and a write version.
New optional fields are ignorable only when the manifest marks them optional;
unknown required semantics fail safely.

Migration runs on typed staged records and produces a deterministic report. The
original Genome is immutable and retained. CI maintains golden fixtures for
every supported version, including hostile and truncated inputs. A future
release never assumes users exported from every intermediate Helix version.

## Verification gates

Genome remains unavailable in production builds until tests cover:

- deterministic logical export and full round-trip for Blueprint and Full
  Clone;
- kill, disk full, read-only target, and cancellation at every publication and
  import checkpoint;
- wrong password, extreme KDF parameters, tampered header/manifest/chunks,
  truncation, duplicate paths, and decompression bombs;
- symlink/hard-link/device/path/mount escapes and numeric-owner attacks;
- port, node ID, user, pool, runtime, and package conflict previews/remaps;
- authorization and reauthentication for user/role/secret import;
- old supported format migrations and new unsupported required features;
- clean-machine recovery without the source host; and
- proof that an import failure leaves existing users, services, and worlds
  unchanged and rollback data intact.

The test corpus must include independently generated fixtures; round-tripping
only Helix's own current writer and reader is not sufficient parser validation.
