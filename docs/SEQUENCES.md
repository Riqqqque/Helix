# Sequence Package and Execution Model

## Status

Sequences are a design only. There is no package schema, registry, parser,
signature verifier, action engine, game definition, or sandboxed worker yet.
Examples in this document define intended boundaries, not accepted syntax.

A Sequence is declarative, versioned knowledge for installing, configuring,
querying, updating, backing up, and running a game or managed service. It is not
a script with a friendly wrapper.

## Security model

Every Sequence is untrusted input, including one fetched from an official
catalog. A signature can establish origin and integrity; it does not prove that
the declared actions are safe. Certification is a tested support level, not a
privilege bypass.

The core rules are:

- no free-form root command or shell-string action;
- deny access outside the assigned instance, runtime, cache, and explicitly
  granted storage handles;
- no network destination that bypasses the outbound URL/SSRF policy;
- no implicit secret access;
- no package install, systemd change, port exposure, license acceptance, or
  executable launch without a named capability and preview;
- every action has bounded input, output, time, resources, cancellation, and a
  declared recovery behavior; and
- permission changes in an update require fresh operator approval.

## Package identity and manifest

A Sequence package has a stable UUID, machine-readable slug, human name,
publisher identity, package version, schema version, supported Helix range, and
content digest. User-facing names never select filesystem paths or systemd
units.

Its signed or authenticated manifest declares:

- game/service identity and support tier;
- supported OS, CPU architectures, server versions, loaders, and dependencies;
- package origin and signature/provenance evidence;
- required and optional capabilities;
- typed install, validate, launch, stop, query, update, backup, and recovery
  plans;
- configuration schemas and versioned migrations;
- port purpose/protocol/default without claiming availability;
- filesystem datasets and whether they are executable, mutable, cache, world,
  config, log, or temporary;
- resource recommendations with their evidence and limits;
- backup consistency strategy and restore validators;
- health/readiness/crash-loop signals; and
- external assumptions with a validation timestamp.

All collections and strings have explicit maximum sizes. Unknown required
fields or actions reject the package. Canonical bytes used for signatures are
defined by the eventual format ADR and covered by cross-implementation vectors.

## Capability vocabulary

Capabilities are narrow and parameterized. Initial categories include:

- `network.fetch` for listed origins and schemes;
- `cache.read` and `cache.populate` for content-addressed objects;
- `instance.files.read`, `instance.files.write`, and
  `instance.files.executable` for declared dataset roots;
- `runtime.acquire` for a named, version-constrained runtime;
- `ports.reserve` for declared protocols/counts;
- `service.define` and `service.control` for the Sequence's stable instance
  unit only;
- `backup.consistency` for reviewed save/pause/stop hooks;
- `secret.use:<purpose>` for a scoped credential handle; and
- `host.package.request` only as an explicit owner-reviewed host change, never
  an automatic Sequence action.

The install UI displays why each capability is required, the concrete roots,
origins, ports, units, and credentials it covers, and which functionality is
lost if an optional grant is denied. Permission grants are stored by package
identity, version range, capability, scope, and approving actor.

## Typed actions

The first engine accepts a small action set, for example:

- fetch a typed HTTPS URL into the content-addressed cache with expected size
  and digest/signature evidence;
- verify digest, signature, file type, or a version-specific manifest;
- create a relative directory under an opened dataset root;
- extract a reviewed archive format through the safe extraction worker;
- copy/reflink/link a verified cache object into an instance staging tree;
- render a schema-validated template into a crash-safe staged file;
- reserve a port transactionally;
- acquire a named runtime through a separate runtime provider;
- generate a systemd unit from a Helix-owned typed model;
- invoke a game executable with a vector of bounded arguments and controlled
  environment; and
- run a typed console/query protocol with explicit success parsing and timeout.

There is no action shaped like `shell: "..."`, `exec_root: String`, arbitrary
systemd unit text, caller-chosen absolute destination, or caller-chosen
environment inheritance. A future advanced native hook would be a separately
installed, high-trust sidecar with an explicit security label, not a loophole in
the Sequence parser.

Arguments remain an array passed directly to an executable; they are never
joined into a shell command. Executable paths come from verified runtime/cache
handles or the instance executable dataset. Environment variables are built
from an allowlist; secrets use scoped descriptors/files where possible rather
than command lines.

## Download and URL policy

Every fetch declares allowed origins, redirect policy, maximum compressed and
logical bytes, expected media type, and integrity evidence. Helix parses and
revalidates every redirect, blocks loopback/link-local/private/cloud-metadata
targets unless an administrator explicitly grants the exact private endpoint,
bounds DNS/time/response headers, and does not silently inherit privileged proxy
configuration.

Prefer publisher signatures or upstream hashes. If upstream offers no
independent digest, Helix may record the TLS-fetched bytes' digest for caching
and reproducibility, but the UI labels origin verification as unavailable. A
cached digest proves byte identity, not publisher trust.

Content-addressed objects are immutable after verification. Cache publication
uses a temporary file, size limit, streaming digest, fsync, atomic rename, and
parent fsync. Concurrent fetches deduplicate by lease. Failed or unverified
objects never receive the trusted digest path.

## Files and archive extraction

Sequence paths are normalized logical paths beneath declared dataset roots.
Resolution is descriptor-relative and race-safe. Display names and manifest
slugs do not become directory names.

Extraction runs in a resource-limited one-shot worker and rejects absolute
paths, `..`, duplicates after normalization, symlink/hard-link escapes, devices,
FIFOs, sockets, setuid/setgid bits, capabilities, attacker numeric ownership,
unsupported sparse forms, excessive depth/count/size/ratio, and writes through
replaced mounts. It extracts to a new operation staging root and publishes only
after complete verification.

If a game legitimately requires a link, the Sequence declares a typed link
between two handles already proven to be within the same instance. Imported
archive links are never trusted directly.

## Install state machine

An installation proceeds through durable states:

```text
Planned -> Authorized -> Reserved -> Downloading -> Staged
        -> Validated -> Publishing -> ServiceDefined -> Ready
```

Each transition records inputs, expected outputs, progress, and recovery
disposition in the operation ledger. Reservations cover instance ID, storage
pool, port, runtime, and required headroom. Publishing occurs only after all
downloads, licenses, configuration, and target compatibility validate.

Cancellation releases safe reservations and leaves no successful instance
record. After a crash, idempotent actions resume; published content rolls back or
remains quarantined according to the last durable checkpoint. A service is not
started until its definition, ownership, resources, executable, config, and
ports all match the committed instance state.

## Runtime and service boundary

The Sequence describes requirements; a runtime provider resolves them against
current official sources. Java, Wine, SteamCMD, and containers are optional and
downloaded only when an approved instance needs them. A Sequence cannot claim a
stale URL or Java rule is permanently correct. Resolution evidence and time are
recorded.

Managed games run as independent systemd services, preferably with distinct
service users and cgroups. They are not child processes whose survival depends
on `helixd`. Units are generated from a typed template with explicit executable,
argument vector, working directory, environment sources, writable paths,
restart policy, startup timeout, stop signal/protocol, resource controls, and
log policy.

The Sequence cannot weaken the Helix daemon/broker sandbox. An instance needing
an unusual capability receives a visible per-instance exception and a lower
security classification.

## Configuration

Configuration fields have a versioned schema, type, bounds, default provenance,
beginner/advanced visibility, sensitivity flag, and validation strategy. The UI
may render forms, but the server repeats validation. Raw advanced editing uses
the same authorization, revision, atomic-write, parse, preview, and rollback
path as forms.

Sequence updates migrate configuration through explicit transformations.
Unknown fields are preserved when the owning format permits it; they are never
dropped merely because the UI does not render them. A migration previews the
semantic diff and creates a restore point before publication.

## Backup consistency and restore validation

A Sequence declares one honest consistency level and typed hooks. Hooks may send
a bounded game-console command, wait for an exact acknowledgement, pause, stop,
or request a filesystem snapshot. Cleanup is structured like `finally`: save
state and the prior running state are restored on success, failure, timeout, or
cancellation.

The Sequence also declares restore validators: required files, parseable config,
world/save markers, compatible server version, ownership policy, and optional
offline game-specific check. A checksum-valid tree is not labeled usable if
these validators fail.

## Updates and rollback

An update is a new operation, never in-place overwrite:

1. refresh current external version assumptions from official sources;
2. verify the new Sequence and artifact provenance;
3. show changed permissions, dependencies, config migrations, ports, and
   compatibility;
4. create a policy-required verified backup/restore point;
5. stage binaries and configuration separately;
6. stop only if the tested update strategy requires it;
7. publish, start if previously running, and run health checks; and
8. restore the old package/config/data if validation fails.

Automatic updates respect the instance policy (`automatic`, `ask`, `manual`)
and never convert a breaking loader/mod/plugin transition into a silent update.
Rollback retains the old verified executable and Sequence until its window
expires.

## Registry, signatures, and support levels

Catalog metadata is untrusted network data and never executable. A package
signature is checked against an explicit trust root and package identity.
Revocation and trust-root rotation are part of the registry protocol.

Support labels mean:

- **Certified:** the exact lifecycle matrix has current Helix-run evidence;
- **Stable:** maintained and tested, but not the full certification matrix;
- **Experimental:** known gaps are shown before install; and
- **Community:** publisher/community maintained with explicit provenance and no
  implied Helix certification.

Installing from a local file or community source is supported only through the
same parser, limits, permission review, and operation ledger.

## Required validation

The engine remains unavailable for production game creation until tests cover:

- schema/golden fixtures for every supported package version and unknown
  required actions;
- signature/digest mismatch, revocation, rollback, and origin confusion;
- command/argument/environment injection and proof no action reaches a shell;
- SSRF, redirect, DNS, proxy, oversized download, truncation, and cache races;
- archive traversal, links, devices, bombs, sparse files, permissions, and mount
  replacement;
- disk full, read-only storage, process kill, cancellation, and restart at every
  install/update checkpoint;
- port/storage/runtime reservation races;
- permission grant/update/revocation and cross-instance isolation;
- consistency-hook timeout/failure with guaranteed resume/save cleanup;
- config migration, failed service start, health failure, and full rollback;
  and
- real install/start/query/stop/update/backup/restore cycles on current supported
  game/API versions using official documentation.

Fuzz targets include the manifest, action decoder, version expressions, path
normalizer, URL policy, archive metadata, configuration migration, and protocol
response parsers.
