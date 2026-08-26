# Installation Model

## Current status

The checksummed local release-bundle lifecycle is **SCOPED LIFECYCLE TESTED — PUBLIC SUPPORT BLOCKED**. It includes transactional package-file install/upgrade, explicit package-file rollback, and conservative uninstall scripts. It is not a supported production installer.

Ubuntu 24.04 GitHub Actions passed the declared Rust 1.88/stable builds and one scoped systemd package lifecycle in [CI run 32956742213](https://github.com/Riqqqque/Helix/actions/runs/32956742213) for commit [`fdbbc0a`](https://github.com/Riqqqque/Helix/commit/fdbbc0aeb7b353069c74fe5186d1d48fd65b66ca). The lifecycle covered archive verification, fresh install, owner claim, protected API and authentication flows, selected file modes and unit hardening, forced-crash restart, clean stop/start, full doctor, verified state backup, secret-redaction canaries, modified-bundle manifest rejection, repeat install, explicit package-file rollback, and data-preserving uninstall.

This is not clean-VM, cross-version-upgrade, schema-downgrade, complete fault-injection, low-disk/power-loss, reference-performance, platform-support, signed-release, or supported-installer evidence. CI first verified the hosted runner's `/usr/share` ancestor was root-owned with its unusual `0777` mode, then normalized it to the conventional root-owned `0755` baseline so Helix's production path checks ran unchanged.

Do not use development binaries as a production installation, and do not publish a one-line installer until clean-install, upgrade, rollback, uninstall, permission, interruption, and fault-injection tests pass on supported Ubuntu releases. The bundle checksum detects accidental or local bundle modification; it is not a signature or proof of publisher authenticity.

## Supported environment policy

The first supported environment will be a documented set of 64-bit Ubuntu Server releases using systemd and cgroup v2. Exact releases and architectures must be chosen from current upstream support information and tested before publication. “Linux” by itself is not a useful compatibility claim.

The package must not require these technologies for Helix itself:

- PostgreSQL, MySQL, MariaDB, Redis, MongoDB, or Elasticsearch;
- Docker, Podman, or Kubernetes;
- nginx or Apache;
- Node.js, Python, PHP, Java, Wine, or SteamCMD.

Some may later be installed or managed for an explicitly selected feature. Node.js remains a frontend build dependency and is not installed on the target host.

## Installation identities

The package creates a dedicated unprivileged system account and group named `helix` unless distribution policy requires a different name. The account:

- has no interactive password or login shell;
- owns writable Helix state, cache, and instance roots;
- cannot modify packaged executables or unit files;
- is not added broadly to administrative groups;
- receives only the filesystem and socket access required by enabled features.

`helixd` runs as this account. A future `helix-privd` runs separately with a smaller root-owned unit and socket. Installing Helix must not make the main web daemon root.

## Local package layout

| Path | Owner and mode intent | Purpose |
| --- | --- | --- |
| `/usr/bin/helixd` | root-owned, not service-writable | Daemon |
| `/usr/bin/helixctl` | root-owned, executable | Administrative CLI |
| `/usr/share/helix/web/` | root-owned, read-only | Hashed static frontend assets |
| `/etc/helix/helix.toml` | root/Helix-readable, not world-writable | Non-secret administrator configuration |
| `/var/lib/helix/state/` | `helix`, `0700`; database/backup files `0600` | Critical database and migration snapshots |
| `/var/lib/helix/metrics/` | `helix`, `0700`; database files `0600` | Replaceable metrics database |
| `/var/lib/helix/instances/` | `helix` or deliberately delegated | Stable-ID instance data |
| `/var/lib/helix/keys/` | `helix`, `0700`; key files `0600` | Fallback key material once implemented |
| `/var/cache/helix/downloads/` | `helix`, bounded | Content-addressed download cache |
| `/run/helix/` | runtime-created, restrictive | Local sockets and transient state |
| `/var/lib/helix-package/rollbacks/` | root-owned, `0700`, not service-writable | Package-file rollback snapshots and exact manifests |

Packaging must use tmpfiles or equivalent declarative mechanisms where appropriate. Permissions are asserted in installation tests, not assumed from the process umask.

Storage-pool roots may live elsewhere. The installer records them only after checking the resolved path, filesystem, ownership model, free space, and mount behavior. It never derives a trusted path from a display name.

## systemd units

The foundation package provides `helixd.service`. It should:

- run as the dedicated account;
- use an explicit configuration path;
- restart on unexpected failure with bounded backoff;
- support readiness and watchdog integration only after correctly implemented;
- apply practical filesystem, capability, syscall, and namespace hardening without breaking documented features;
- set sensible file-descriptor and task limits;
- stop gracefully with a bounded timeout;
- send logs to journald;
- avoid declaring network readiness it does not actually require.

Future units are separate:

- `helix-privd.socket` and `helix-privd.service` for typed privileged requests;
- template or transient units for independent game workloads;
- transient `helix-worker` units/scopes for expensive jobs;
- `helix-strandd.service` only when an installed Strand needs the optional host.

Managed game units must not use `PartOf=helixd.service` or another relationship that stops them when the dashboard restarts. Package uninstall and upgrade tests must explicitly verify this independence.

## Secure first run

An installation must not expose an unclaimed administrator form to every client that can reach the port. The intended flow is:

1. install files and create the service account and directories;
2. create a short-lived, single-use bootstrap capability using a cryptographically secure source;
3. start `helixd` in setup state;
4. print the one-time token through `helixctl setup-token` without writing the secret into world-readable logs;
5. let the user select a bind and TLS/reverse-proxy mode deliberately;
6. create the owner account and recovery material;
7. revoke the bootstrap capability atomically;
8. refuse further bootstrap attempts unless an explicit local recovery procedure resets setup.

The current package automates steps 1–4 and starts the loopback-only setup
surface. The current owner flow consumes the token atomically. Bind/TLS selection
and recovery-material creation in steps 5–6 are planned, not current installer
behavior.

Until TLS, the remote authentication/cookie boundary, and trusted-proxy behavior
are implemented and independently validated, runtime configuration enforces
loopback-only access. Remote setup is not implemented. A local tunnel may be
used only as an explicit operator-controlled development measure; binding or
proxying the bootstrap setup surface to a network is not acceptable.

The setup flow detects host facts but does not install Docker, Java, Wine, SteamCMD, or another optional dependency without explaining why and receiving confirmation.

## Local bundle lifecycle

Status: **SCOPED LIFECYCLE TESTED — PUBLIC SUPPORT BLOCKED**.

Builds created by `scripts/build-release.sh` include `install-local.sh`, `rollback-local.sh`, `uninstall-local.sh`, and their shared `package-common.sh`. Every regular payload file, including these scripts, is listed in the bundle's `SHA256SUMS`. These are local tools; they do not download remote code and make no signature claim.

> [!CAUTION]
> These scripts write fixed paths under `/usr`, `/etc`, and `/var`, create the
> `helix` system account and group, and manage `helixd.service`. Until installation
> is supported, run them only on an isolated disposable Ubuntu test host.

From an extracted release bundle:

```bash
sudo ./install-local.sh
sudo ./rollback-local.sh --snapshot snapshot-YYYYMMDDTHHMMSSZ-XXXXXXXX
sudo ./uninstall-local.sh
```

`install-local.sh` and `rollback-local.sh` accept only exact local inputs. All lifecycle operations take the non-blocking process lock `/run/lock/helix-package.lock`. Install verifies the complete bundle manifest, rejects symlinks and special files, stages package content into sibling paths, validates the staged binaries and assets, and creates a root-owned package snapshot before stopping `helixd`. Regular files use same-directory atomic replacement; the web tree is exchanged through validated same-directory staging and a retired sibling. If any step after the stop fails, the installer restores the prior package files and the prior enabled/active service state automatically.

Snapshots contain an exact allowlisted manifest plus checksums for only these package targets:

- `/usr/bin/helixd`;
- `/usr/bin/helixctl`;
- `/usr/share/helix/web`;
- `/usr/lib/systemd/system/helixd.service`;
- `/usr/lib/sysusers.d/helix.conf`;
- `/usr/lib/tmpfiles.d/helix.conf`.

The explicit rollback command accepts only a snapshot ID with the generated fixed format, resolves it beneath the root-owned rollback directory, verifies its exact manifest, ownership, file types, allowlist, and checksums, and creates a safety snapshot before applying it. Rollback never touches `/etc/helix`, the service-account database, `/var/lib/helix`, `/var/cache/helix`, instances, backups, or other application data.

This is package-file recovery, not an application-state or database rollback. The daemon performs forward SQLite migrations when it first opens state; the package scripts do not yet coordinate restoring a pre-migration database when an older binary cannot read the new schema. Migration code creates and verifies a no-clobber state snapshot before changing an existing schema, but a future migration-aware lifecycle must join that snapshot to package rollback and validate the complete interrupted-upgrade contract described under Upgrades before it can claim full rollback.

The installer records whether `/var/lib/helix/state/helix-state.db` existed before tmpfiles or a Helix CLI can create it. After the service account, data roots, configuration, and package files are installed—but while `helixd` is stopped—it runs a read-only `helixctl status` probe as `helix` against the configured data directory. It generates an owner setup token only when both the original fixed-path database was absent and the configured-state probe found no readable installation. A normal existing installation therefore does not replace its bootstrap token or fail merely because an owner already exists. Customized, ownerless, or damaged state that cannot be classified safely requires the manual flow below.

For a genuinely fresh install, the installer runs `helixctl setup-token` as the unprivileged `helix` account before the first service start. Token stdout goes directly to the invoking terminal once; the script does not assign it to a variable or write it to a file. If the token command fails, package files and the prior service state roll back, while any application state created by the command remains preserved for explicit recovery.

To replace an unclaimed setup token manually, only before an owner exists, keep the daemon out of the database lease and use this exact order:

```bash
sudo systemctl stop helixd
sudo -u helix helixctl setup-token
sudo systemctl start helixd
```

`--no-start` installs the verified files without starting `helixd`; a previously active service remains stopped by explicit request. A fresh install can still print a 15-minute token, so start the service within that window or use the manual replacement sequence when ready. A normal successful install enables and starts `helixd`, then runs the bounded `helixctl ready --timeout-seconds 20` probe as the service account. That probe requires bodyless liveness, a responsive critical-state setup-status route, and a nonempty compiled HTML shell before installation is accepted. A readiness failure enters the same automatic package-file and prior-service-state rollback path. Failure recovery preserves application state and restores the package/service state captured before the transaction.

## Planned remote verified installer flow

The convenience installer should perform these steps:

1. detect architecture, operating-system release, init system, cgroup mode, required tools, and available storage;
2. reject unsupported environments with a useful explanation and no partial changes;
3. fetch a signed release manifest over TLS;
4. verify manifest signature against a pinned project key;
5. download the exact package or archive;
6. verify size and cryptographic digest before execution or extraction;
7. display the version, source, destination, and planned system changes;
8. install through the native package manager where available;
9. verify files, ownership, service status, readiness, and installed version;
10. display the access and recovery guidance without leaking bootstrap material to logs.

TLS transport alone is not artifact verification. Checksums hosted beside a compromised artifact are not a sufficient trust root. The project must document key rotation and compromise recovery before making signed-update claims.

For security-conscious users, releases should also publish direct package URLs, detached signatures, checksums, and offline verification instructions. Those commands must use real release assets and keys; placeholders must not be presented as runnable installation instructions.

## Configuration

Packaged configuration is read from `/etc/helix/helix.toml` by default and is
limited to 64 KiB before parsing. Environment overrides are limited to
documented deployment needs and must not provide a hidden second configuration
language. Secret values should use protected files or the secret store rather
than command-line arguments or a broad `.env` file.

An invalid configuration prevents readiness and produces a concise diagnostic. Helix does not silently continue with a dangerous default after an administrator supplied an invalid value.

Development overrides may redirect all writable paths into a temporary project-local root. They must be opt-in, reject production-like ambiguous paths, and be clearly reported at startup.

## Network exposure

Installation must make the bind address, port, and TLS boundary explicit. The package does not automatically open a firewall or trust a reverse proxy. If firewall integration is later provided, it is a previewed, typed privileged operation.

Security headers and cookie behavior depend on whether `helixd` terminates TLS or receives verified proxy metadata. Forwarded headers are ignored unless the peer is in an explicit trusted-proxy configuration. Setup and health endpoints reveal minimal unauthenticated information.

## Upgrades

An upgrade is a recoverable operation, not a file overwrite:

1. inspect compatibility and available space;
2. verify the new artifact;
3. retain the previous working package;
4. create a consistent critical-state snapshot;
5. stop only the Helix control-plane processes that require replacement;
6. apply package files and forward-compatible migrations;
7. start and validate readiness, database integrity, assets, and version;
8. retain rollback material until the configured confidence window passes.

Running managed workloads should remain running unless a specific workload update was requested. Migrations must declare whether the previous binary can read the new schema. If not, rollback requires restoring the pre-migration state snapshot as one coordinated procedure.

A future migration-aware lifecycle must detect and reconcile interrupted upgrades. The current local package-file transaction handles ordinary errors and trapped termination signals, while its retained snapshot provides a manual recovery path after an untrappable process or host failure. A package-manager success code alone is not proof that the upgraded service works.

## Uninstall

The local uninstall script stops `helixd`, disables it when it was enabled, and removes only the fixed package-owned executables, packaged static assets, unit, sysusers file, and tmpfiles file. It takes a package snapshot first and automatically restores the package plus prior enabled/active state if a post-stop step fails.

User data is preserved by default:

- `/etc/helix` and administrator configuration;
- the `helix` system account and group;
- state and metrics databases;
- instances, worlds, saves, and configurations;
- `/var/cache/helix` and downloaded cache content;
- backups and storage-pool data;
- recovery material;
- `/var/lib/helix-package/rollbacks` and package recovery snapshots.

A destructive purge is a separate explicit command that inventories exact targets, requires confirmation, refuses broad or unresolved paths, and reports recoverability. Uninstall must not delete adopted services or independently managed game data merely because Helix once observed them.

## Verification matrix

Before publishing installation support, test on clean virtual machines for each supported release and architecture:

- clean installation and first-owner creation;
- exact file ownership and modes;
- daemon crash/restart and systemd watchdog behavior if enabled;
- managed workload survival through daemon restart and upgrade;
- invalid configuration and unavailable storage;
- repeated installer execution;
- interrupted install and interrupted migration;
- upgrade from every supported predecessor;
- rollback with and without schema change;
- uninstall with data preservation;
- explicit purge against a disposable fixture;
- low-disk behavior;
- no-network startup;
- artifact signature and checksum failures;
- port conflict and bind failure;
- reverse-proxy and direct-TLS modes when supported.

The current Windows development host cannot validate systemd state transitions, `flock` contention, Linux ownership and mode enforcement, sibling rename behavior, interruption recovery, low-disk handling, or injected failures at every post-stop step. One GitHub-hosted Ubuntu 24.04 run covers the scoped lifecycle named above, but it does not validate every interruption/fault boundary, cgroup behavior, Debian packaging, clean-host variation, or reference resource usage. Clean Ubuntu/systemd virtual machines are required before marking installation support as available.
