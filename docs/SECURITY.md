# Security Architecture and Threat Model

## Status and security claim

This is the founding security design. The current foundation does set a
loopback-only default, strict TOML parsing, bounded API bodies/timeouts, request
IDs, a restrictive initial CSP and related response headers, and distinct
SQLite durability domains. The owner/session foundation and portable
recoverable-secret store now have focused unit tests. These are useful defenses,
not a complete or independently audited security boundary.

The secret-store foundation is not wired to production master-key delivery,
rotation, recovery, daemon startup, or HTTP authorization. There is no
implemented privileged broker, extension sandbox, update verifier, or archive
parser. Existing unit, API, Windows release-binary coverage, and one scoped
Ubuntu 24.04 package/systemd lifecycle are not an independent or comprehensive
target-Linux security assessment. Runtime configuration currently rejects every
non-loopback listen address. Remote binding remains unavailable until all
authentication and remote-access gates exist. Nothing in this document is a
claim that the current repository is safe to expose to a network or trust with
production data.

Helix may call a control "implemented" only when its code path, failure path,
tests, and deployment configuration are linked from `PROGRESS.md`. It may call a
release "secure" only after the packaged release gates and an independent review
are satisfied.

## Security objectives

Helix is a local-first Linux control plane with authority over valuable data and
potentially destructive host operations. Its default posture is:

- the current `helixd` runs without root and rejects non-loopback binds; any
  future remote mode must pass its separate exposure gate;
- all authorization decisions are enforced server-side;
- future privileged actions cross a narrow, authenticated, typed local boundary;
- untrusted packages, archives, paths, URLs, UI, and protocol frames are
  hostile inputs;
- recoverable secrets are encrypted and verification-only secrets are hashed;
- managed game services survive a panel or extension crash; and
- security failure is explicit and recoverable, not converted into a success
  message.

## Assets

The highest-value assets are:

- owner/admin credentials, password hashes, sessions, API tokens, MFA and
  recovery material;
- the installation master key and backup/Genome encryption keys;
- RCON, Steam, cloud-backup, notification, SSH, and third-party credentials;
- users, roles, permissions, audit history, policies, schedules, and instance
  definitions;
- worlds, saves, game configuration, mods/plugins, backups, and restore points;
- Sequence and Strand packages, update metadata, downloaded executables, and
  installer/release artifacts;
- privileged broker requests and host-control results; and
- availability of `helixd`, independent game services, storage, and recovery
  workflows.

Metrics, caches, and console buffers are lower durability domains, but they can
still contain private data or be weaponized to exhaust memory and disk.

## Trust boundaries

1. **Browser to `helixd`:** remote, attacker-controlled HTTP/WebSocket input.
2. **`helixd` to state/metrics storage:** unprivileged process crossing into
   durable data with different corruption consequences.
3. **Planned `helixd` to `helix-privd`:** low-privilege control plane requesting
   narrow host changes from a root broker. This boundary is not implemented.
4. **Helix to managed processes:** game servers and plugins may be compromised
   and must not inherit Helix authority.
5. **Helix to workers:** archives, backup, restore, compression, and imports are
   high-resource, parser-heavy jobs.
6. **Helix to Strands/sidecars:** third-party extension code with explicitly
   granted capabilities.
7. **Helix to the network:** downloads, update sources, webhooks, remote Vaults,
   and game catalogs are untrusted even over TLS.
8. **Backup/Genome import:** data crosses machines and may be malicious,
   corrupted, stale, or stolen.
9. **Local users and filesystem:** another local account may race paths, replace
   mounts, inspect process state, or attempt socket access.

## Threat actors and required response

| Threat actor | Representative attack | Required controls | Residual boundary |
| --- | --- | --- | --- |
| Unauthenticated remote attacker | First-owner takeover, login guessing, SSRF, traversal, oversized frames | Loopback-first bind, single-use local enrollment, rate limits, strict parsing, origin/CSRF checks, size/time limits | A remotely exposed service still has parser and denial-of-service risk. |
| Malicious low-privilege user | IDOR, role escalation, reading another instance, inducing privileged action | Server-side object-and-action authorization, stable tenant/instance IDs, reauthorization in jobs, audit records | A granted game-console capability may be powerful inside that game. |
| Malicious Strand | Secret theft, host filesystem/network access, CPU or memory exhaustion | Deny-by-default capabilities, separate optional host, quotas, capability handles, isolated UI, revocation | Native sidecars are trusted code, not a strong sandbox. |
| Malicious Sequence | Command injection, hostile download, writes outside instance root | Typed actions, schema limits, reviewable permissions, digest/provenance verification, constrained path handles | A user-authorized Sequence can modify its assigned instance. |
| Malicious uploaded archive | Zip slip, symlink/hardlink escape, decompression bomb, device creation | Staged extraction, descriptor-relative paths, type rejection, byte/count/depth/ratio quotas | Parsing bugs remain possible; fuzzing and worker isolation are required. |
| Compromised game plugin/mod | Read game credentials, corrupt world, pivot to host | Per-instance service user, minimal filesystem rights, cgroup limits, no Helix socket/key access | Helix cannot guarantee application-consistent data from a compromised game. |
| Local non-root user | Read keys/DB, connect to privileged socket, symlink race | restrictive modes, peer credentials, no-follow path resolution, service isolation | Kernel/local privilege-escalation bugs are outside Helix's control. |
| Stolen backup/Genome | Offline password attack, disclosure, rollback to stale policy | authenticated encryption, calibrated KDF, no raw master key, signed/authenticated manifest, explicit age/version display | Weak user passphrases reduce offline resistance. Metadata may remain visible by format design. |
| Compromised update source | Malicious Helix/game/Strand binary | pinned trust roots, signed metadata/artifacts, hashes, rollback protection, provenance, explicit trust display | A compromised trusted signing key requires revocation and a recovery release. |
| Root attacker | Read process memory, replace binaries, alter kernel/audit/storage | honest limitation, minimize secret lifetime, external/off-host verification where possible | Unrestricted root on the running host is outside Helix's confidentiality and integrity guarantee. |

## CIA analysis by subsystem

### Critical state

- **Confidentiality:** strict file modes; ciphertext for recoverable secrets;
  ordinary API responses and support bundles omit secrets.
- **Integrity:** SQLite constraints, `foreign_keys=ON`, `WAL`,
  `synchronous=FULL`, migrations with verified pre-migration backup, audit trail.
- **Availability:** one bounded writer, short transactions, busy handling,
  integrity checks, low-disk reserve, explicit recovery rather than automatic
  destructive repair.

### Metrics and logs

- **Confidentiality:** redact credentials, tokens, addresses when configured,
  and user-supplied command content where it could contain secrets.
- **Integrity:** telemetry is never authoritative for permissions or desired
  service state.
- **Availability:** separate metrics database, bounded retention/rings/frames,
  journald-first Helix logging, circuit breakers for failing collectors.

### Managed games and files

- **Confidentiality:** per-instance identities and roots; no access to Helix
  keys or another instance by default.
- **Integrity:** descriptor-relative paths, atomic Helix-owned config writes,
  configuration revisions, verified downloads, staged updates and restore.
- **Availability:** systemd-owned workloads survive `helixd`; cgroup resource
  limits, crash-loop protection, backup consistency hooks.

### Vault and Genome

- **Confidentiality:** authenticated encryption for secret-bearing or full
  clone artifacts, independent recovery secret, no plaintext secret manifest.
- **Integrity:** self-describing versioned manifest, cryptographic hashes,
  authenticated metadata, corrupted artifacts rejected before publication.
- **Availability:** off-host copies, retention, cancellation checkpoints,
  staged restores, periodic test restore, no dependency on the original host.

### Extensions and automation

- **Confidentiality:** no secret access without a named capability; use scoped
  handles rather than bulk secret export.
- **Integrity:** versioned manifests, capability review on install/update,
  signed artifacts for origin/integrity without treating a signature as safety.
- **Availability:** optional process boundary, time/memory/CPU/I/O limits,
  bounded queues, cancellation and circuit breaking.

## Identity, onboarding, and authentication

Before an owner exists, `helixd` remains loopback-only and does not accept an
unauthenticated remote owner-creation request. `helixctl setup-token` creates a
cryptographically random, single-use, short-lived enrollment secret, stores
only its hash, and displays it locally. Owner creation and token consumption are
one transaction so concurrent requests cannot create multiple first owners.

Passwords use Argon2id from a reviewed library. Initial parameters follow the
documented OWASP basis, are stored with each hash, are bounded against
resource-exhaustion input, and are upgraded after successful authentication
when policy changes. Calibration on supported low-end Ubuntu hardware remains a
public-release gate. Passwords are never encrypted for recovery.

Sessions use at least 256 bits of randomness in opaque bearer tokens. Only a
keyed or cryptographic hash is stored in the database. Cookies are `HttpOnly`,
have an explicit `SameSite` policy, a narrow path, rotation after login and
privilege change, idle and absolute expiry, and `Secure` whenever HTTPS is in
use. Session revocation, account lock/disable, password change, and role change
invalidate affected sessions. High-impact operations can require recent
reauthentication.

Normal post-convergence state retains at most 64 session rows per user. One login
deletes at most 256 imported excess rows and returns a generic retryable
maintenance response if more work remains. At the cap, eviction, password
rehash/reset state, random token creation, new-session insertion, stale pruning,
and audit success are one transaction; RNG, audit, or commit failure preserves
the prior credentials and sessions.

Cookie-authenticated mutations require CSRF protection and strict Origin/Host
validation. WebSocket upgrades validate authentication and Origin before
allocating large buffers. Login and recovery endpoints have per-source and
per-account rate limits with bounded state; limits do not permit attacker-driven
unbounded database growth.

The current synchronizer-token rotation is compare-and-swap: two requests using
the same proof cannot both commit replacement proofs. Every protected route,
including reads, logout, and rotation itself, requires the cookie plus the
current in-memory proof during loopback HTTP operation.

MFA is not yet implemented. The account/session model must reserve versioned,
encrypted TOTP data, hashed single-use recovery codes, and WebAuthn credentials
without weakening password-only deployments. UI text must not imply MFA exists
until it does.

## Authorization

The server is authoritative. Hiding a control in the frontend is never an
authorization mechanism. Each request and deferred job checks both:

1. the actor's capability, such as `system.services.control`, `files.write`,
   `backups.restore`, or `strands.install`; and
2. the actor's scope over the concrete node, instance, storage pool, backup, or
   user object.

Jobs store the initiating actor and authorization snapshot, but re-check current
authorization before every privileged or destructive step. Revoked authority
does not remain usable through a queued job. Denials reveal no cross-scope
object existence beyond what the caller may list. Permission changes are
audited and covered by positive and negative tests.

## Planned privilege boundary

The current `helixd` runs unprivileged and is restricted to loopback. Its
dedicated systemd service identity and selected sandbox properties passed one
scoped Ubuntu 24.04 package lifecycle. Broader supported-host, configuration,
permission, and fault matrices remain open. It has no implemented API for
rebooting the host, managing services, modifying package state, or browsing
unrestricted user files.

`helix-privd` does not exist yet. The intended design is a small, separately
packaged root broker, preferably activated by a systemd Unix socket. It would
have no TCP listener, shell, plugin runtime, archive parser, frontend, or general
file manager. Its socket would have restrictive ownership and mode, and the
broker would verify kernel-provided peer credentials and accept only the
expected Helix service identity.

The planned protocol is versioned, length-bounded, and typed. Operations name
stable objects and constrained parameters, for example
`RestartManagedUnit(UnitId)`;
there is no `run_root_command(String)`, arbitrary unit name, free-form command
line, or caller-selected absolute write path. The broker independently repeats
authorization-independent safety validation. For path work it opens the trusted
root and resolves the target by file descriptor, closing validation/use races.

The future broker systemd units must use a reviewed sandbox profile: capability
bounding, `NoNewPrivileges`, filesystem protections, private temporary space,
device and address-family restrictions, system-call filtering where compatible, and
explicit writable paths. A hardening score is useful evidence, not proof.
Required exceptions are documented per operation instead of disabling the
sandbox globally.

## Secrets and key management

The portable implementation in `helix-secrets` uses XChaCha20-Poly1305
envelopes. Every 1-to-65,536-byte record gets a fresh random 32-byte DEK and
24-byte data nonce. The installation master key wraps that DEK with a separate
fresh 24-byte nonce. Versioned canonical binary associated data binds record
type and stable identity, installation, record and wrap revisions, and
master-key identity as appropriate. Authentication failure is a hard error;
corrupted ciphertext, a substituted row, or manipulated revision is never
returned as partial plaintext.

The version-3 state schema stores master-key identity/lifecycle metadata, an
authenticated master-key check record, ciphertext, nonces, and wrapped DEKs. It
never stores the random installation master-key bytes. The strict versioned
credential is accepted only as redacted, zeroizing in-memory bytes. The crate
does not load credentials from configuration, environment variables, CLI
arguments, or files, and it has no HTTP dependency or plaintext response type.
Reads invoke a caller closure over a zeroizing plaintext buffer; the caller can
still deliberately copy it and must preserve this boundary.

Only initial key creation and verification are implemented. Preferred Ubuntu
delivery remains a systemd credential, but package/service wiring and effective
credential lifetime have not been tested. Protected fallback key files, key
rotation/rewrapping, TPM2 sealing, and independent recovery envelopes are not
implemented. Reserved lifecycle columns are not evidence that those workflows
exist. See [ADR 0005](adr/0005-recoverable-secret-storage.md).

Verification-only API and recovery tokens are hashed rather than encrypted.
Plaintext secrets are redacted from structured logs, errors, audit payloads,
metrics labels, tracing spans, support bundles, crash reports, command lines,
process environments where avoidable, and ordinary serialization. Secret types
must not derive or implement revealing debug/display output. Memory is zeroized
where practical, while documentation remains honest that copies may exist in
allocators, TLS stacks, kernels, and a compromised process.

The intended recovery contract is that loss of the machine and master key is
recoverable only if the user previously configured an independent recovery
secret that wraps required key material. That workflow does not exist yet, so a
current database backup without its separately protected master credential is
unrecoverable. A future recovery secret must never be stored beside the backup.

## Network and web security

The initial bind is `127.0.0.1` and `::1`. Listening on LAN or public addresses
requires an explicit configuration change and a visible warning. Remote use
requires TLS directly or through a configured reverse proxy. Helix trusts
forwarded headers only from enumerated proxy addresses and validates their
syntax; otherwise it uses the socket peer and direct scheme.

Responses set a restrictive Content Security Policy, frame policy, MIME
protection, referrer policy, and appropriate cache controls. HSTS is emitted
only on a correctly configured HTTPS origin. Strand UI cannot force the core
policy to allow global `unsafe-eval` or arbitrary same-origin script.

All request bodies, uploads, WebSocket frames, decompressed payloads, list page
sizes, recursion, and job queues have limits. Errors do not include secrets,
filesystem roots, SQL, or internal command lines. Server-generated download
names are encoded safely rather than copied into headers.

## URLs, downloads, and supply chain

Outbound requests are SSRF-sensitive. Each initial URL and redirect is parsed
as a typed URL, restricted to allowed schemes, resolved with bounded DNS, and
checked against loopback, link-local, multicast, private, Unix-socket, and cloud
metadata targets unless a named administrator policy explicitly permits that
destination. Redirects are revalidated. Proxy environment variables are not
silently inherited by privileged workers.

Sequence, Strand, game, runtime, and Helix downloads record source, expected
version, size limits, digest/signature evidence, and retrieval time. Prefer
upstream signatures or hashes. TLS without an authenticated artifact digest
protects transport but is not equivalent to supply-chain verification. An
unverified artifact is labeled honestly and cannot become a trusted executable
through cache reuse alone.

Helix releases require pinned CI actions, dependency review, checksums,
signatures, SBOM, and provenance where practical. Trust-root rotation,
revocation, rollback protection, and an offline recovery release process must be
defined before automatic self-update is enabled.

## Audit, privacy, and support data

Chronicle records actor, action, stable target, result, authorization scope,
request correlation ID, and trustworthy server timestamp. It never stores raw
passwords, bearer tokens, MFA seeds, private keys, or full secret-bearing
request bodies. Audit append-only application semantics and optional hash
chaining can make tampering evident; they do not make records immutable against
root. Off-host forwarding is required for stronger root-compromise evidence.

Schema v4 applies a fixed first-stage retention contract to the implemented
authentication and session events:

- the newest 1,024 events are always retained, even after they exceed the time
  window;
- rows outside that newest floor are eligible after 90 days;
- 50,000 rows is the steady-state ceiling; and
- one audited write or daemon-start maintenance pass removes at most 256 rows.

Cleanup uses a deterministic order and converges old imported overages across
bounded transactions. When the record ceiling applies, older denied events are
removed before older success/error events, while the newest 1,024 remain
protected. Ordinary SQL update/delete attempts remain blocked by the
append-only triggers. The internal pruner temporarily replaces the delete guard
inside one immediate SQLite transaction; failure or process interruption rolls
the deletion and trigger change back together. An idle database does no cleanup
work and cannot grow; its next startup or audited write applies the 90-day
cutoff. A large pre-policy backlog can temporarily remain above the target while
the 256-row passes converge, but every audited write above the ceiling removes
at least as many rows as it adds.

This is local retention, not tamper evidence. Export, operator-selected holds,
hash chaining, off-host forwarding, broader operator events, and independent
recovery evidence remain future work.

Support bundles are previewed before creation, default to the minimum necessary
data, apply structural redaction, exclude keys/databases/worlds by default, and
are themselves treated as sensitive. Redaction tests use seeded canary secrets
across logs, config, errors, filenames, database fields, and frontend payloads.

## Root and platform limits

Helix cannot defend confidentiality or integrity against an attacker with
unrestricted root on the running host. Such an attacker can read process memory,
replace binaries and keys, alter the kernel, forge local audit records, and
modify data before backup. Helix also cannot compensate for storage hardware or
filesystems that falsely report durable flushes, a compromised firmware/kernel,
or lack of host disk encryption after physical theft.

The product can still reduce consequence through least privilege, short secret
lifetimes, signed releases, recovery copies, per-instance isolation, explicit
root boundaries, and off-host verification. These are defense in depth, not an
"unhackable" claim.

## Required security verification

Before public recommendation, automated and manual testing must cover:

- first-owner races, password parameter limits, session fixation/rotation,
  revocation, CSRF, WebSocket Origin, and brute-force controls;
- positive and negative permission tests for every route and job transition;
- IDOR across users, instances, nodes, pools, backups, and operations;
- traversal, symlink/mount races, malicious filenames, and archive bombs;
- command injection and proof that privileged RPCs cannot express arbitrary
  commands or paths;
- SSRF including redirects, alternative IP encodings, DNS rebinding, and proxy
  behavior;
- XSS and CSP behavior in core and Strand UI;
- token/secret leakage through logs, errors, tracing, metrics, audit, support
  bundles, environment, process arguments, and APIs;
- malformed Sequence, Strand, Genome, archive, and protocol inputs with fuzzing;
- oversized frames, queue backpressure, CPU/memory/disk exhaustion, and
  cancellation; and
- install/upgrade file modes, systemd hardening, update verification, rollback,
  backup corruption, and recovery drills on supported Ubuntu releases.

The packaged release gate remains closed while any required control is merely
documented, mocked, waived without rationale, or covered only by Windows or one
scoped hosted lifecycle.

## External assumptions to revalidate

- systemd execution and credential controls:
  <https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html>
- systemd socket activation and socket ownership:
  <https://www.freedesktop.org/software/systemd/man/latest/systemd.socket.html>
- Linux constrained path resolution:
  <https://man7.org/linux/man-pages/man2/openat2.2.html>
- SQLite durability and WAL behavior:
  <https://www.sqlite.org/pragma.html> and <https://www.sqlite.org/wal.html>

The implementation must pin minimum supported versions and verify the effective
runtime behavior rather than assuming every Linux distribution exposes the
same controls.
