# Security Architecture and Threat Model

## Status and security claim

Helix is a private alpha. It has real authentication, capability checks, a
typed Linux broker, configured-root file controls, native game management,
private gateway policy, and focused security tests. It has not completed an
independent security review or a public-network deployment review.

Do not expose this build directly to the public internet or use it as the only
protection/copy for valuable data. A passing unit or mock test is evidence for
that narrow behavior, not proof that a root-capable control plane is generally
secure.

Current major gaps include:

- no MFA;
- no complete production master-key delivery, rotation, or independent
  recovery workflow;
- no signed/digest-pinned Helix release and rollback channel;
- no package rollback or complete interruption/conffile/dpkg-recovery matrix
  for the guarded selected-candidate update job;
- no full clean-host broker, filesystem-race, power-loss, UFW, reboot, and game
  lifecycle matrix;
- no broker peer-credential check beyond the configured Unix-socket filesystem
  ownership/mode boundary; and
- no independent authentication, broker, or public-exposure review.

Broad/unattended package upgrades and Helix self-update therefore remain
unavailable. Exact selected APT candidates have an explicit guarded path, but it
makes no rollback claim. Public release status remains blocked in
[PROGRESS.md](../PROGRESS.md).

## Security objectives

Helix is a local-first Linux control plane with authority over host state and
valuable game data. Its rules are:

- `helixd` remains unprivileged;
- root operations cross only the length-bounded typed Unix-socket broker;
- the broker exposes no general shell, caller-selected binary, or arbitrary
  systemd-unit operation;
- every protected HTTP operation checks authentication, CSRF proof, and its
  named capability on the server;
- state-changing HTTP requests validate Origin/Fetch Metadata before side
  effects;
- configured roots and exact opaque identities are revalidated at the broker;
- AMP remains a separate manager and does not inherit native Helix semantics;
- a failed or unverified action is not converted into a success message; and
- game workloads and console capture remain independent of the browser.

## Assets

High-value assets include:

- owner credentials, password hashes, sessions, CSRF proofs, and future MFA or
  recovery material;
- installation keys and encrypted secret records;
- broker socket/configuration and every privileged request/result;
- users, capabilities, preferences, audit history, and instance definitions;
- worlds, server configuration, plugins/mods, console history, backups, and
  deleted-backup recovery records;
- AMP service credentials and AMP-managed instance identity;
- downloaded runtime/marketplace artifacts and update metadata; and
- availability of `helixd`, `helix-privd`, Docker, AMP, native game servers,
  storage, and recovery workflows.

Metrics, package caches, and console archives are lower-authority data, but can
still expose private content or exhaust resources.

## Current trust boundaries

1. **Browser to private gateway/`helixd`:** attacker-controlled HTTP input,
   cookies, CSRF proofs, JSON, paths, names, and search terms.
2. **`helixd` to critical/metrics state:** an unprivileged process writes data
   with different corruption and confidentiality consequences.
3. **`helixd` to `helix-privd`:** a length-bounded typed frame crosses a
   group-protected Unix socket into a root process.
4. **Broker to configured storage roots:** filesystem names, links, mounts,
   permissions, and concurrent replacement can be hostile.
5. **Broker to Docker/native games:** server artifacts and plugins/mods may be
   compromised and must not receive Helix authority.
6. **Broker to AMP:** a separately protected credential reaches a loopback AMP
   API whose responses may be unavailable, ambiguous, or maliciously shaped.
7. **Helix to UFW/systemd/APT tools:** root-owned host interfaces whose output
   and current state must be parsed and verified without a shell.
8. **Helix to upstream catalogs:** Minecraft, Java, Modrinth, and other remote
   metadata/artifacts are untrusted even over TLS.
9. **Backup/import boundaries:** archives may be corrupt, stale, stolen, or
   crafted to escape intended roots.
10. **Browser/`helixd` to `helix-terminald`:** a short-lived authenticated
    WebSocket bridges into one unprivileged Linux user's PTY over a separate
    group-protected, peer-credential-checked Unix socket.
11. **Local root/kernel:** unrestricted root can read memory, replace binaries,
    forge local evidence, and bypass application policy. This is outside
    Helix's confidentiality/integrity guarantee.

## Authentication and browser boundary

Before an owner exists, `helixctl setup-token` creates a random, single-use,
short-lived enrollment token and stores only its hash. Owner creation and token
consumption are atomic so concurrent requests cannot create two first owners.

Passwords use Argon2id with bounded PHC parsing and prospective-password rules.
Parameters are stored with the hash and can be upgraded after a successful
login. Supported-hardware calibration and compromised-password screening remain
release work.

Sessions use opaque random bearer tokens; only a cryptographic hash is stored.
The browser cookie is `HttpOnly`, host-only, `SameSite=Strict`, and scoped to
`/`. A reviewed HTTPS deployment must add the appropriate Secure/host-prefix
policy.

Every protected route requires the session cookie and the current session-bound
`X-Helix-CSRF` proof. The frontend keeps that proof in memory, so a reload
requires login instead of accepting a cookie-only request. CSRF rotation is
compare-and-swap: one proof cannot successfully create two replacement proofs.

State-changing routes validate configured Origin and incompatible Fetch Metadata
before authorization-dependent body processing. Login, bootstrap, preferences,
and expensive storage analysis have bounded operation-specific rate state.

Changing the owner password invalidates older sessions through the account
authentication version. MFA does not exist and the UI must not imply it does.

## Authorization

The server is authoritative. Hiding a control in Preact is never an authorization
mechanism.

Current capabilities include:

- `system.view`, `system.settings.write`, and `system.power`;
- `dashboard.customize` and `users.manage`;
- `storage.files.read`, `storage.files.manage`, and `storage.analyze`;
- `network.firewall.read` and `network.firewall.write`;
- `system.packages.read` and `system.packages.write`;
- `terminal.open`;
- `games.view`, `games.manage`, and `games.backups.manage`.

HTTP authorization is checked before a broker request. The broker then repeats
authority-independent validation: root/config identity, configured roots,
opaque IDs, exact container names, bounded values, expected state, and
operation-specific concurrency.

The current owner model has capability foundations but not a mature multi-user,
per-instance delegation UI. Do not present future fine-grained grants as
implemented.

## Implemented privileged broker

`helix-privd` is a separately deployed root process. It has no TCP listener,
frontend, plugin runtime, or general command executor. Its configuration must be
absolute, root-owned, private, and bounded. Its Unix socket is created with
restricted group access.

The protocol is a closed, versioned Rust enum framed with a bounded length.
Operations name typed values such as an opaque server/rule/backup/operation ID,
bounded port range, or configured-root path. There is no
`run_root_command(String)` request.

The service example uses a systemd sandbox and explicit writable paths. Native
content roots remain narrow. Selected APT updates, UFW changes, and recurring
reboot schedules deliberately require write access to reviewed OS-managed
trees, and package/UFW work cannot keep kernel-tunable and set-ID creation
blocks that would make legitimate host mutations fail halfway through. Those
operations therefore carry real root-host authority even though the request
protocol remains typed and closed. The profile and its target-host exceptions
still need full review. The current
socket boundary relies on filesystem ownership/mode and service-group isolation;
kernel peer credentials are not yet independently checked by the broker, so the
complete peer-identity gate remains open.

## Files and storage

File operations are restricted to configured roots. Linux implementations use
descriptor-relative traversal and reject unsupported types, traversal, and link
escapes at the operation boundary. Text reads/writes, directory listings, and
analysis results have explicit limits.

Writes use revision or current-state checks where applicable. Deletion moves an
entry into configured recoverable trash rather than claiming irreversible
removal succeeded.

The storage analyzer is bounded and cancellable, does not follow symbolic links,
and avoids holding a complete large tree in memory. Mount replacement,
permission changes, extremely deep/wide trees, low disk, and live multi-terabyte
behavior still require disposable target-host testing.

Dashboard preferences, including Home layout names, notes, widget settings, and
colors, are ordinary application state. They are revisioned and included in
verified critical-state backups, but they are not encrypted field-by-field.
Do not use a Home note as a password, private-key, or token vault.

## Native games, AMP, console, and backups

Helix-native Minecraft instances use Docker and configured state/instance/backup
roots. Docker calls use structured argument arrays rather than shell strings.
Per-instance operation guards prevent incompatible actions from running at the
same time.

Native console input is one bounded line. Output capture runs independently of
the browser and writes protected rotating segments with byte/file limits.
Cursor history is bounded; persistence means retained across browser closes and
boots, not unlimited retention.

Native backup deletion creates an opaque trash record and moves only the exact
known archive/metadata pair. Undo restores only the matching protected record
within policy. This does not replace off-host backups or prove restore after
disk loss.

AMP credentials must be root-owned/private, and the AMP endpoint must remain on
loopback. AMP responses are bounded and strictly interpreted. An unavailable or
ambiguous AMP action fails; Helix does not adopt or rewrite AMP instances during
discovery.

## Host power and start on boot

The start-on-boot operation accepts one boolean and changes Docker restart policy
only on the exact configured Helix dashboard and gateway container names. It
does not enable/disable Docker or change current container running state. A
future container recreation may reapply Compose policy, which the UI states
explicitly.

One-shot whole-host reboot requires:

- capability `system.power`;
- exact current-hostname confirmation;
- a 10–300 second delay;
- explicit disruption acknowledgement;
- active-player and running-job preflight; and
- an opaque cancellable systemd transient-timer operation.

Recurring reboot stores one verified daily or weekday local-time schedule and
reports the host timezone and next activation. It uses the same hostname,
acknowledgement, and workload preflight boundaries. Helix never chains package
work to an automatic reboot.

Tests mock these calls and never reboot the host. Disposable live Linux
validation remains mandatory before relying on this control.

## Network and UFW

The Network page deliberately separates:

- sockets bound on this host;
- Docker host/container port publication;
- UFW installed/active/default/rule state; and
- outside reachability.

Outside reachability is currently `unverified`. Docker DNAT can take a different
path than UFW INPUT. Helix never labels a listener or matching rule as proof a
router, upstream firewall, CGNAT boundary, or ISP permits traffic.

Rule mutation is available only when UFW is installed, active, and verified.
Helix rules use an exact opaque UUID comment and a durable private ownership
record. Create, delete, restore, and crash-pending reconciliation compare exact
rule bodies and record before/after evidence under one mutation lock.

A separate activation flow can enable an installed inactive UFW only after the
operator supplies an exact confirmation and a TCP port that Helix independently
observes listening. It stages and records an exact allow rule for that SSH port,
enables UFW, verifies the active state and exact rule, and attempts to restore
the inactive state if verification fails. Helix never resets UFW, changes its
defaults, deletes an unowned rule, or opens a router. Live UFW mutation testing
is still a release gate and must use a disposable host/rules.

## Packages and Helix updates

Opening the package page reads dpkg/APT inventory and a simulation without
refreshing lists or mutating dpkg. A separate explicit refresh job can run
`apt-get update`. A separate selected-candidate job accepts bounded exact
name/installed/candidate tuples, rejects holds and changed candidates, requires
download headroom and disruption acknowledgement, re-runs a no-removal/no-new-
package simulation, preserves existing conffiles, serializes APT work, and
verifies every final installed version. Bounded job logs are retained.

This is not transactional package rollback. Power loss, maintainer-script
failure, dpkg partial configuration, service disruption, and unusual conffile
states still require the distribution's recovery tools and a disposable test
matrix. Helix never auto-reboots after package work.

## Optional host terminal

The terminal is intentionally not part of the root broker. `helix-terminald`
runs as one configured non-root Linux account, starts a fresh login shell in
that account's home, and has no path to the broker group. Its private socket is
owned by a distinct `helix-terminal` group and accepts only a process whose
kernel-reported effective UID exactly matches the pinned dashboard UID. Root is
not an accepted configured peer UID.

Opening a browser terminal requires capability `terminal.open`, the current
session/CSRF proof, and a fresh verification of the current Helix password. The
API then sets a random 30-second, one-use, session-bound, path-scoped HttpOnly
cookie; the ticket is never returned in a URL or JSON body. WebSocket setup
requires an exact same Origin and one exact `helix-terminal-v1` subprotocol.

The daemon caps concurrent sessions and frame/input sizes, clears the inherited
environment to a minimal allowlist, and kills the PTY when transport ends. Helix
audits password rejection plus session opened/closed/failed events, but does not
store terminal commands, keystrokes, output, or environment values. Normal host
policy still applies inside the shell: `sudo` may prompt and can increase
authority if that Linux user is allowed to use it. Long-running work belongs in
`tmux` or a supervised service because a browser disconnect ends the PTY.

Helix self-update stays unavailable until releases are signed and digest-pinned
and the updater has staging, compatibility checks, configuration/data backup,
health verification, and automatic rollback. `git pull` is not an acceptable
updater.

## Marketplace and supply chain

Native server/runtime and Modrinth requests use HTTPS with constrained expected
hosts and bounded responses. Marketplace profiles restrict software kind,
loader, game version, and server/client environment before installation.
Paper/Purpur receive compatible plugins, Folia requires Folia content, and
Fabric receives compatible server-side mods. Vanilla, Forge, and NeoForge do
not receive a fake marketplace path.

The separate “Start with a modpack” flow accepts only opaque Modrinth IDs and
ordinary server settings from the browser. The broker re-resolves metadata,
permits only listed stable server-capable Fabric releases with one unambiguous
`.mrpack`, restricts downloads to exact Modrinth API/CDN hosts without
redirects, verifies the Modrinth-declared archive SHA-512 and index-declared
SHA-1/SHA-512, and enforces archive/file/count/byte/expansion/path/depth/time/disk
bounds. Traversal, links, devices, case collisions, and Helix-owned-path writes
are rejected. Fresh same-filesystem staging is activated only after validation;
failure removes the exact incomplete container, manifest, and instance.

Server-optional and client-only files are excluded and counted. The result is a
server-safe subset, not byte-for-byte full-pack parity. Forge, NeoForge, Quilt,
unknown loaders, CurseForge, and broad modpack behavior remain unsupported.

TLS and declared hashes protect specific transport/integrity properties; they
do not prove an artifact is safe. Download size, provenance evidence,
destination, replacement policy, restart behavior, and rollback still require
review per integration. Broad dependency behavior, non-Fabric modpacks,
CurseForge, and every upstream failure mode are not supported claims.

Helix releases additionally require signed artifacts, checksums, SBOM/provenance
where practical, trust-root rotation, key-compromise recovery, and rollback
protection before automatic update.

## Secrets and key management

`helix-secrets` implements XChaCha20-Poly1305 record envelopes with a fresh DEK
and nonce, master-key wrapping, versioned associated data, bounded plaintext,
and zeroizing/redacted access. The database stores wrapped keys and ciphertext,
not the installation master-key bytes.

Production systemd credential delivery, effective in-memory lifetime, key
rotation/rewrapping, fallback-key policy, TPM use, and an independent recovery
envelope are not complete. A database backup without its separately protected
master credential may be unrecoverable. Reserved schema fields are not proof a
workflow exists.

Passwords and verification-only tokens are hashed, not encrypted for recovery.
Secret types avoid revealing debug/display output, while documentation remains
honest that copies can still exist in allocators, libraries, kernels, or a
compromised process.

## Audit, privacy, and support data

Current Chronicle records bounded authentication/session events without raw
passwords, bearer tokens, keys, or full secret-bearing requests. State retention
keeps a newest-event floor and prunes older data in bounded transactions.

This is local append-only application behavior, not tamper evidence against
root. Broader operator actions, export, holds, hash chaining, off-host
forwarding, and a reviewed support-bundle generator remain future work.

Helix has no required cloud account and no advertised telemetry pipeline. Local
weather, Modrinth, Minecraft/runtime, and optional AMP requests still disclose
the information necessary to those configured services.

## Private network and Tailscale

Development defaults to loopback. The container gateway can be bound to an exact
private address/port with explicit host, origin, and client-CIDR policy. An
optional second private entry point can be used with an already configured
Tailscale route.

Helix does not install, enable, authenticate, or reconfigure Tailscale. Do not
trust a wildcard hostname/origin/CIDR or the entire Tailscale carrier-grade NAT
range just because one expected node uses Tailscale. Public-network exposure is
not supported by the private alpha.

## Root and platform limits

Unrestricted root on the running host can read process memory, alter binaries,
keys, files, firewall state, timers, containers, and local audit data. A
compromised kernel/firmware or storage device can also lie about isolation and
durability. Helix cannot make those conditions safe.

Least privilege, typed operations, explicit writable roots, bounded work,
recoverable deletion, independent backups, signed releases, and off-host
evidence reduce consequence; they do not make Helix “unhackable.”

## Required security verification

Before public recommendation, automated and manual testing must cover:

- first-owner races, password limits, session rotation/revocation, CSRF,
  Origin/Host, remote onboarding, brute force, and MFA decisions;
- positive/negative capability tests and IDOR across every protected object;
- broker peer identity, malformed frames, concurrency, cancellation, and proof
  that no request can express an arbitrary command/path/unit;
- traversal, symlink/mount replacement, malicious filenames, permissions,
  low-disk, archive bombs, and interrupted storage/backup/restore operations;
- disposable live UFW, reboot, start-on-boot, Docker, and AMP matrices;
- SSRF/redirect/DNS/proxy behavior for every outbound integration;
- malicious marketplace/runtime metadata and artifact provenance;
- secret leakage through logs, errors, metrics, audit, API/UI, process
  arguments/environment, backups, and support data;
- oversized responses/queues, CPU/memory/disk pressure, long-running console
  retention, and recovery after process loss;
- clean install, upgrade, rollback, uninstall, modes, systemd hardening, and
  state/key recovery on supported Ubuntu versions; and
- signed release and self-update rollback drills.

The release gate stays closed while a required control is merely documented,
mocked, covered only by one host, or waived without an explicit narrow reason.

## External assumptions to revalidate

- systemd execution: <https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html>
- systemd transient units: <https://www.freedesktop.org/software/systemd/man/latest/systemd-run.html>
- Linux constrained path resolution: <https://man7.org/linux/man-pages/man2/openat2.2.html>
- UFW behavior: <https://manpages.ubuntu.com/manpages/jammy/man8/ufw.8.html>
- APT simulation: <https://manpages.ubuntu.com/manpages/jammy/man8/apt-get.8.html>
- SQLite durability/WAL: <https://www.sqlite.org/pragma.html> and <https://www.sqlite.org/wal.html>

Minimum supported versions and effective target-host behavior must be tested;
documentation defaults are not runtime proof.
