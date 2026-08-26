# Helix Roadmap

## Purpose

This roadmap defines dependency order and exit criteria. It is not a promise of dates and does not prove a feature exists. Verified state belongs in `PROGRESS.md`, with links to builds, tests, measurements, and operational evidence. The exact resumption point belongs in `NEXT.md`.

Status vocabulary:

- **NOT STARTED**
- **DESIGNING**
- **IMPLEMENTING**
- **IMPLEMENTED — UNVALIDATED**
- **TESTED**
- **BLOCKED**
- **COMPLETE**

A phase may contain work at several states. The status below is the phase's lowest honest summary, not an average and not a marketing label.

## Sequencing rules

1. Security, integrity, recovery, and resource bounds are part of a feature, not a later polish pass.
2. The smallest complete vertical slice is preferred over broad fake screens.
3. Host mutation waits for the typed privileged broker.
4. Serious game hosting waits for a tested local backup and restore path.
5. Game support waits for a lifecycle test on the real current software and platform.
6. Remote Vault and Genome claims wait for successful fresh-machine restoration.
7. Third-party Strands wait for mature authentication, authorization, jobs, state, and APIs.
8. Optional dependencies are added only when their feature is enabled and measured.
9. A later phase may be researched early, but it does not bypass the earlier safety dependency.

```text
Foundation
   |
Secure core + real host visibility
   |
Typed host administration
   |
Safe files and storage
   |
Local Vault and tested restore
   |
Independent native game execution
   |
Validated Sequence engine
   |
Certified game integrations
   |
Genome and remote Vault
   |
Optional Strand ecosystem
   |
Release hardening
```

## Phase 0 — Foundation

**Status: BLOCKED**

Build the smallest real Helix application and the contracts that prevent later rewrites:

- public-repository structure and contribution documents;
- Rust workspace with `helix-core`, `helix-auth`, `helix-config`, `helix-state`, `helix-secrets`, `helix-system`, `helix-api`, `helixd`, and `helixctl`;
- lightweight frontend workspace compiled to static assets;
- architecture, storage, security, performance, recovery, API, Sequence, Strand, Genome, and operational documentation;
- typed configuration and production/development path policy;
- critical SQLite initialization, migrations, integrity checks, and bounded execution;
- common error and structured logging behavior with redaction;
- versioned API, liveness/readiness, and a real host snapshot;
- static frontend shell that renders real API data and honest degraded states;
- graceful startup and shutdown;
- formatting, lint, unit/integration tests, dependency policy, and CI;
- systemd, Debian packaging, installer, upgrade, uninstall, and rollback skeletons.

Exit criteria:

- clean workspace build, Clippy, tests, frontend lint/test/build, and secret scan;
- no fabricated production data or successful no-op controls;
- a clean supported Ubuntu VM installs a verified artifact and serves the compiled UI;
- service user, directories, modes, readiness, restart, uninstall preservation, and rollback behavior are tested;
- initial Linux RSS, CPU, startup, binary, frontend, API, and SQLite baselines are recorded;
- `PROGRESS.md` and `NEXT.md` make every unvalidated item explicit.

Scaffolding or a page rendering does not complete this phase.

## Phase 1 — Secure core, Lattice, and Pulse

**Status: IMPLEMENTING**

Deliver the first useful authenticated dashboard:

- race-safe owner enrollment;
- password hashing, session lifecycle, logout, revocation, and CSRF/origin protection;
- roles and capability-based authorization foundation;
- encrypted recoverable-secret storage and key lifecycle;
- read-only host discovery for CPU, memory, uptime, storage, and network;
- adaptive live and historical metrics with isolated failure behavior;
- versioned event stream and reconnect behavior;
- Lattice dashboard layouts and widgets stored in critical state;
- Midnight, OLED, Nebula, Light, and System theme foundations;
- responsive, keyboard-accessible, reduced-motion-aware UI;
- real loading, empty, stale, denied, degraded, and disconnected states.

Exit criteria:

- authentication and authorization positive/negative tests pass;
- bootstrap cannot be claimed remotely or reused;
- session and secret redaction tests pass;
- all displayed host values come from an identified source of truth;
- metrics failure leaves authentication and critical administration available;
- disabled metrics and unused widgets have measured negligible background cost;
- the dashboard meets recorded initial bundle and responsiveness budgets.

## Phase 2 — Host administration

**Status: NOT STARTED**

- versioned local protocol and socket-activated `helix-privd`;
- systemd service inventory and narrowly authorized controls;
- process inspection and resource views;
- operating-system update discovery and previewed application;
- quick host controls with confirmation and reauthentication;
- Chronicle audit/change history;
- health center, storage detail, network detail, and Port Planner;
- support bundle generation with tested redaction.

Exit criteria include peer-credential enforcement, path/target revalidation, absence of arbitrary command execution, broker fuzzing, systemd failure tests, and complete audit attribution.

## Phase 3 — Files and storage safety

**Status: NOT STARTED**

- root-scoped file manager with descriptor-relative path safety;
- bounded streaming upload and download;
- staged, quota-limited archive inspection and extraction;
- lazy-loaded editor for supported text formats;
- crash-safe atomic Helix-owned configuration writes;
- configuration revision, diff, and rollback;
- storage-pool definitions, mount identity, capacity, health, and placement;
- low-disk protective mode and operation preflight.

Exit criteria include traversal, symlink-race, mount-replacement, decompression-bomb, permission, disk-full, and process-kill tests. User data must never be overwritten by an unverified staged result.

## Phase 4 — Local Vault

**Status: NOT STARTED**

- integrate the existing verified database-only critical-state snapshot
  primitive into Vault catalogs, retention, and restore workflows;
- filesystem backup plans with consistency hooks;
- local and second-disk Vault destinations;
- schedules, retention, pre-change backup hooks, manifests, and checksums;
- staged restore, integrity validation, cleanup, and recovery UI;
- restore-confidence records based on real drills;
- backup strategy assistant and recovery-key workflow.

Exit criteria:

- backup and restore survive interruption and restart;
- corrupted or incomplete data is rejected;
- state, configuration, and representative game data are restored into a clean fixture;
- retention never deletes the only good copy or an in-use object;
- “verified” means checksum and structural verification, while “restore tested” requires an actual restore drill.

## Phase 5 — Native game execution

**Status: NOT STARTED**

- instance domain model and stable filesystem identities;
- native systemd runtime backend;
- independent workload lifetime through `helixd` restart and upgrade;
- resource limits, ports, environment, graceful stop, health, logs, and console;
- durable install/update/control jobs and incompatible-operation locks;
- crash-loop protection and evidence-based “why did it stop?” diagnostics;
- adoption workflow that inspects and backs up before changing an existing service.

First prove the lifecycle with a simple non-game fixture. Do not use a synthetic fixture as proof that a real game is supported.

## Phase 6 — Sequence engine

**Status: NOT STARTED**

- versioned declarative schema, parser, and validator;
- identity, platform, dependency, runtime, config, update, backup, and query models;
- closed typed installer-action set;
- capability and resource preview;
- checksum and provenance policy;
- configuration-schema rendering with raw expert access;
- cancellation, idempotency, recovery, and compatibility behavior;
- schema fixtures, negative tests, fuzzing, and reference Sequence.

Sequences never gain arbitrary shell commands or caller-selected privileged paths. A valid schema is not proof that its external download or game lifecycle still works.

## Phase 7 — Minecraft

**Status: NOT STARTED**

Initial certified targets:

- current Vanilla and Paper server workflows;
- verified Java runtime selection and download;
- explicit EULA acknowledgement;
- settings, raw configuration, console, players, worlds, backups, restore, and updates;
- lifecycle and recovery tests on supported Ubuntu releases.

Then expand based on current upstream support:

- Purpur, Fabric, NeoForge, and modpack workflows;
- Modrinth integration and terms-compliant CurseForge handling;
- plugin/mod compatibility, updates, rollback, and world safety;
- Bedrock;
- Velocity, Folia, and advanced network management.

Every implementation revalidates current game, loader, mapping, Java, API, dependency, and licensing facts from official sources. Support levels are **Certified**, **Stable**, **Experimental**, or **Community**, each with visible evidence and last-tested versions. Multiple real server types must pass install, start, query, stop, restart, update, backup, and restore before Minecraft can be called broadly complete.

## Phase 8 — V Rising

**Status: NOT STARTED**

Implement and test the current Ubuntu hosting workflow, including SteamCMD, compatibility tooling if upstream still requires it, JSON configuration, ports, updates, query, logs, saves, backup, restore, and graceful shutdown.

Do not freeze an August 2026 compatibility assumption or describe Wine-based execution as native Linux support.

## Phase 9 — Additional games

**Status: NOT STARTED**

Add one deep, tested integration at a time. Candidate order is informed by user demand and current feasibility, not the size of a catalog:

- Valheim;
- Palworld;
- Project Zomboid;
- Rust;
- Terraria/tModLoader;
- Factorio;
- Satisfactory;
- 7 Days to Die;
- CS2.

Each integration documents external dependencies, configuration coverage, console/query behavior, saves, update safety, backup consistency, restore, current limitations, and a real lifecycle test matrix. A generic process wrapper may be useful but is not first-class game support.

## Phase 10 — Genome

**Status: NOT STARTED**

- versioned self-describing manifest and compatibility policy;
- Blueprint exports without bulk game data;
- Full Clone exports with explicit user-selected content;
- authenticated encryption and independent recovery material;
- signed or authenticated metadata, checksums, and provenance;
- import inspection and dry-run plan;
- exact and portable clone modes;
- hardware, path, dependency, storage, and port conflict planning;
- fresh-machine migration and recovery.

Exit requires successful export and import on two clean machines, malicious/corrupt input rejection, wrong-key behavior, partial-write cleanup, and an honest display of content that cannot be legally or technically bundled.

## Phase 11 — Remote Vault

**Status: NOT STARTED**

- backend abstraction proven by an off-host provider;
- encrypted, resumable, bandwidth-aware transfer;
- credential lifecycle and recovery-key separation;
- repository health, retention, pruning, and integrity checks;
- interruption, offline, bad-credential, quota, stale-catalog, and corruption behavior;
- full restore to a fresh machine without the original host.

A remote upload is not disaster recovery until a restore drill succeeds.

## Phase 12 — Strands

**Status: NOT STARTED**

- versioned manifest and SDK;
- capability declaration, review, grant, revocation, and audit;
- optional out-of-process sandbox host;
- CPU, memory, I/O, storage, network, and call limits;
- namespaced data and mediated secret handles;
- isolated, lazy-loaded UI contribution model;
- package provenance, update review, compatibility, and reference Strand;
- crash, timeout, hostile-message, and resource-exhaustion tests.

The base daemon must show no Wasm-runtime residency when no Strand requires it. Native sidecars are explicitly trusted code with a stronger warning and separate policy.

## Phase 13 — Hardening and public-release gate

**Status: NOT STARTED**

Perform:

- independent security review and remediation;
- fuzzing of all high-risk parsers and protocols;
- fault injection, disk-full, unclean-shutdown, and power-loss testing;
- memory, CPU, I/O, startup, database, API, and frontend regression analysis;
- clean install, upgrade, rollback, uninstall, and artifact-integrity matrices;
- backup, corrupted-backup rejection, state recovery, and fresh-machine restoration drills;
- accessibility and mobile core-flow review;
- privacy, telemetry opt-in, and support-bundle redaction review;
- multiple real game lifecycle matrices;
- operator, recovery, contributor, and API documentation review.

Public use is blocked while a major control is fake, a security or recovery path is untested, performance exceeds its budget without a documented exception, or release provenance cannot be verified.

## Cross-cutting work

These tracks run throughout every phase:

- **Performance:** baselines, budgets, profiles, bounded resources, and regression evidence.
- **Security:** threat-model updates, dependency review, secret scanning, permission tests, and least privilege.
- **Recovery:** operation ledgers, snapshots, rollback, drills, and honest restore confidence.
- **Accessibility:** semantic structure, keyboard behavior, focus, contrast, scaling, motion, and screen readers.
- **Privacy:** local-first operation, opt-in telemetry, data inventory, retention, redaction, export, and deletion.
- **Supply chain:** locked dependencies, minimal features, RustSec/deny policy, frontend audit, signed release metadata, and provenance.
- **Documentation:** architecture and ADR alignment, source-of-truth maps, operator instructions, and truthful status.

## Deferred by design

The following do not belong in the early product:

- Kubernetes or a Helix-specific orchestration cluster;
- a hypervisor;
- a database server dependency;
- a mail or DNS server implementation;
- a proprietary cloud requirement;
- distributed consensus;
- an embedded AI model;
- a general-purpose root command runner or workflow language.

Remote nodes may be considered only after stable identifiers, protocols, permissions, jobs, recovery, and single-host operation have proven durable.

## Immediate unblocked work

1. Validate the existing Phase 0 package, service, filesystem policy, recovery
   behavior, and MSRV on clean supported Ubuntu hosts.
2. Finish production master-key delivery, key rotation, and independent recovery
   around the implemented portable secret envelope.
3. Add bounded Pulse history and a versioned event stream without creating an
   idle polling cost when no dashboard is connected.
4. Persist versioned Lattice layouts and complete formal accessibility and
   representative mobile review of the authenticated dashboard.
5. Expand Chronicle beyond bounded authentication/session retention to broader
   operator events, export, holds, tamper evidence, and off-host forwarding
   before remote exposure, then perform the independent authentication and
   deployment-boundary review.

If Linux validation infrastructure is unavailable, continue portable implementation and tests, mark Linux-dependent gates **BLOCKED** or **IMPLEMENTED — UNVALIDATED**, and leave exact reproduction steps in `NEXT.md`.
