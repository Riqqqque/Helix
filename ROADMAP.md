# Helix Roadmap

## Purpose

This roadmap defines dependency order, not dates. It does not prove a feature
exists. Current evidence belongs in [PROGRESS.md](PROGRESS.md), and the next
concrete validation work belongs in [NEXT.md](NEXT.md).

Status vocabulary: **NOT STARTED**, **DESIGNING**, **IMPLEMENTING**,
**IMPLEMENTED — UNVALIDATED**, **TESTED**, **BLOCKED**, **COMPLETE**.

## Sequencing rules

1. Security, recovery, failure behavior, and resource bounds ship with a
   feature, not after it.
2. A real narrow vertical slice is better than a broad fake control surface.
3. Root authority crosses only the typed local broker; Helix never grows a
   general root shell endpoint.
4. Existing game services remain independent. AMP is integrated, not absorbed.
5. A server type is supported only after a real lifecycle and restore matrix on
   the stated host/game versions.
6. A listener, container publication, firewall rule, and outside reachability
   are different facts.
7. Updates stay unavailable until exact artifacts/candidates can be staged,
   verified, health-checked, and recovered safely.
8. Remote and extension claims wait for mature identity, authorization, audit,
   recovery, and package provenance.
9. Disabled optional features must not add permanent polling or heavyweight
   runtime cost.

## Phase 0 — Private-alpha foundation

**Status: TESTED, public release BLOCKED**

Implemented:

- Rust workspace, locked dependency policy, CI, formatting, lint, tests, and
  frontend asset budgets;
- unprivileged `helixd`, versioned HTTP API, restrictive web defaults, and
  separate critical/replaceable state domains;
- owner bootstrap, login, sessions, CSRF, capability checks, audit foundation,
  and account updates;
- compiled responsive Preact dashboard with honest loading, denied, degraded,
  unavailable, and disconnected states;
- private container and systemd/broker deployment assets; and
- `AGPL-3.0-or-later` repository and package metadata.

Still required for a public release:

- clean supported-host install/upgrade/rollback/uninstall matrices;
- signed artifacts and provenance;
- independent security review;
- full recovery/fault/performance/accessibility evidence; and
- public-network exposure review.

## Phase 1 — Dashboard, Home, and live host visibility

**Status: TESTED**

Implemented:

- Overview, Home, Storage, Network, Host, Terminal, Servers, Hooks, and Settings pages;
- CPU, memory, swap, uptime, disks, interfaces, routes, services, processes,
  and bounded listener views;
- multiple exportable Home layouts with draggable/resizable widgets for clock,
  host, servers, storage, weather, paged notes, and safe HTTP(S) shortcuts;
- synchronized revisioned preferences, customizable metric cadence, reorderable
  navigation, and bounded color controls;
- System, Midnight, OLED, and Light themes; and
- responsive keyboard/reduced-motion behavior and lazy feature chunks.

Remaining work includes historical metrics/events, broader operator audit,
formal screen-reader review, representative devices, and reference Linux
performance.

## Phase 2 — Typed host, storage, and network administration

**Status: IMPLEMENTED — UNVALIDATED**

Implemented through `helix-privd`:

- configured-root file browsing, bounded text editing, creation, rename,
  recoverable trash, and cancellable size analysis;
- Docker/broker integration status and Helix-only resource/error reporting;
- exact dashboard/gateway restart-policy control for start on boot;
- immediate and recurring whole-host reboot with hostname confirmation,
  disruption acknowledgement, timezone evidence, and workload preflight;
- Linux listener/process, Docker publication, game-port, and UFW inventory;
- exact Helix-owned UFW allow-rule create/delete/Undo plus a separate confirmed
  SSH-safety activation path, without reset/default changes;
- APT/dpkg inventory, explicit package-list refresh, and exact selected-
  candidate apply jobs with no rollback or auto-reboot claim;
- bounded Hooks for exact configured services; and
- an optional current-password-gated, peer-UID-checked non-root PTY service.

Exit criteria:

- clean-host peer-credential and systemd sandbox validation;
- traversal/symlink/mount, low-disk, interruption, and cancellation matrices;
- disposable live reboot and UFW tests with unrelated-policy proof; and
- clear support limits per Ubuntu release and architecture.

Selected package application is implemented but stays in the validation phase
until interruption, conffile, and dpkg-recovery matrices pass.

## Phase 3 — Native Minecraft control plane

**Status: IMPLEMENTED — UNVALIDATED**

Current native targets:

- Paper;
- Purpur;
- Folia;
- Fabric; and
- Vanilla.

Implemented:

- Docker-backed isolated instance identities and configured roots;
- one-click creation with explicit EULA acceptance, validated ports, memory,
  player limit, and start-on-boot choice;
- a post-create start-on-boot toggle that changes Docker restart policy without
  starting or stopping the server now;
- start, stop, restart, update, health, performance, settings, files, logs,
  console, backups, restore, and background jobs;
- per-instance operation locks and typed non-shell Docker calls;
- bounded rotating console archives with cursor pagination across retained
  boots;
- restart-required setting metadata; and
- recoverable backup deletion with opaque trash identity and Undo; and
- a narrow Fabric-only Modrinth `.mrpack` creation path with verified declared
  hashes, strict archive bounds, server-safe exclusions, atomic activation, and
  exact rollback.

Exit criteria require real current-version lifecycle, crash, update, backup,
restore, disk-full, and long-running retention matrices on each supported host.
Synthetic high-cardinality tests prove only that Helix stays bounded; they do
not prove a Minecraft player count.

Forge and NeoForge are not supported in this phase.

## Phase 4 — AMP coexistence

**Status: IMPLEMENTED — UNVALIDATED**

The optional AMP bridge uses a separately protected credential and loopback
endpoint. Helix can inventory AMP instances and invoke the bounded AMP actions
it understands. AMP remains the authority for AMP instance files and behavior.

Exit requires versioned live AMP tests, outage/session-expiry/concurrency
matrices, and proof that discovery or failed actions never alter an AMP
instance.

## Phase 5 — Minecraft content marketplace

**Status: IMPLEMENTED — UNVALIDATED**

Implemented:

- Modrinth search and project detail;
- server-software and Minecraft-version filtering;
- Paper/Purpur plugin, Folia plugin, and Fabric server-mod profiles;
- rejection of loader mismatch and client-only Fabric content; and
- bounded background install jobs into the correct instance directory; and
- all-loader modpack preview with creation limited to stable server-capable
  Fabric `.mrpack` releases and explicitly non-parity server-safe results.

Remaining:

- real upstream outage/rate-limit/artifact matrices;
- dependency/conflict/update/rollback behavior;
- broader restart guidance and world-safety evidence;
- Linux extraction/resolver/Docker plus upstream/real-pack validation for
  Fabric `.mrpack` creation; and
- lifecycle validation across supported game versions.

Vanilla has no plugin/mod install path. Forge, NeoForge, Quilt, CurseForge, and
broad/full-parity modpack workflows remain unsupported until each has an
explicit implementation and real matrix. A catalog card is not support.

## Phase 6 — Safe package and Helix updates

**Status: IMPLEMENTED — UNVALIDATED for selected APT candidates; Helix update DESIGNING**

Inventory, explicit list refresh, and exact selected-candidate apply jobs are
implemented. They make no rollback claim and need disposable failure matrices.

A supported package job must continue to provide:

- explicit candidate selection and immediate revalidation;
- dpkg lock, disk, conffile, workload, service, and reboot impact preflight;
- one serialized mutation with bounded durable logs and clear interruption
  boundaries;
- no fake rollback claim and no automatic reboot; and
- clear recovery when apt/dpkg reports partial configuration.

Helix self-update must provide:

- signed and digest-pinned releases;
- staging and compatibility checks;
- configuration/data backup;
- health verification; and
- automatic rollback to the exact previous release.

`git pull` is not a release updater.

## Phase 7 — Vault, recovery, and migration

**Status: NOT STARTED beyond state snapshots and native backups**

Planned:

- local and second-disk backup destinations;
- schedules, retention, manifests, checksums, and consistency hooks;
- staged restore and recovery UI;
- restore-confidence records based on real drills;
- encrypted portable Genome export/import; and
- off-host backup only after a full clean-machine restore succeeds.

A backup is not disaster recovery until a restore drill passes. Helix must
never delete the only verified copy.

## Phase 8 — Additional game integrations

**Status: V Rising IMPLEMENTED — UNVALIDATED; other games NOT STARTED**

Add one deep integration at a time, based on current feasibility and demand.
Candidates include Bedrock, Velocity, Valheim, Palworld, Project Zomboid, Rust,
Terraria/tModLoader, Factorio, Satisfactory, 7 Days to Die, and CS2.

V Rising is a Helix-owned Wine + SteamCMD container path, not a Hub image and
not a host OS Wine install. Create/update/start/stop/backup/restore/remove and
last-instance image teardown are implemented. A live SteamCMD download, Wine
stability, and publisher-support claim remain open. Public UPnP is not offered
for V Rising in this release.

Every integration documents runtime dependencies, ports, config coverage,
console/query behavior, saves, updates, backups, restore, limitations, and a
real current lifecycle matrix. A generic process wrapper is not first-class game
support.

## Phase 9 — Strands and automation

**Status: PARTIAL — installable UI-only Strands**

Owners can pack, review, install, enable, share, and open `helix.strand/1`
UI-only packages. Host calls are metrics, namespaced KV, and allowlisted HTTPS.
There is no Helix-operated store and no package signatures.

Still later:

- portable Wasm / `helix-strandd`;
- native sidecars;
- signed provenance and auto-update;
- automation jobs driven by Strand events.

Native sidecars remain trusted code, not a sandbox.

## Phase 10 — Remote access and public-release hardening

**Status: BLOCKED**

The container gateway can be constrained to a private LAN and can expose a
second explicitly configured private entry point suitable for an existing
Tailscale route. Helix does not install or manage Tailscale.

Before any public recommendation:

- complete independent security review and remediation;
- validate TLS/proxy trust, remote onboarding, cookie policy, MFA, and brute
  force behavior as one exposure boundary;
- fuzz high-risk protocols/parsers;
- complete power-loss, disk-full, corruption, backup, and recovery drills;
- sign releases and prove upgrade rollback;
- complete accessibility/mobile/privacy/support-bundle review;
- pass multiple real game lifecycle matrices; and
- record reference resource/performance limits.

Public use stays blocked while a major control is fake, a recovery path is
untested, a security boundary is unaudited, or release provenance cannot be
verified.

## Deferred by design

Early Helix does not need:

- Kubernetes or a Helix orchestration cluster;
- a hypervisor;
- a separate database server;
- a mail or DNS server implementation;
- a proprietary cloud requirement;
- distributed consensus; or
- a general-purpose root command runner.

Remote nodes can be reconsidered only after single-host identity, protocol,
permissions, jobs, recovery, and upgrade behavior have proved durable.
