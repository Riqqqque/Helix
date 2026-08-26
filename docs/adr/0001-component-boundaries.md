# ADR 0001: Component and Process Boundaries

- **Status:** Accepted
- **Date:** 2026-08-25

## Context

Helix needs to feel like one small local product while handling work with very different privilege, failure, memory, and durability profiles. HTTP requests and lightweight host observations should be cheap and continuously available. Root-level host changes, archive extraction, backup/restore, game processes, and third-party extensions should not share one unrestricted process.

Splitting every feature into a resident service would increase installation complexity, memory, logging, networking, and failure modes. Keeping everything in one daemon would give parser-heavy or privileged features the same blast radius as authentication and orchestration, and linking optional runtimes into the base process would charge every installation for unused capability.

The central product invariant is that `helixd` is a control plane. A running game server must not terminate because the UI, daemon, worker, or extension host restarts.

## Decision drivers

- Low idle memory and effectively zero idle CPU.
- Managed-workload availability independent of the panel.
- Least privilege and a narrow root boundary.
- Isolation of memory-heavy, parser-heavy, and untrusted work.
- Simple installation and diagnosis on one Linux host.
- Reusable domain rules without a distributed-systems architecture.
- Clear ownership of configuration, state, host data, API, and process lifecycle.
- Future compatibility with remote nodes without building clustering now.

## Options considered

### One binary and one crate

One `helixd` binary could contain HTTP, SQL, Linux access, privileged commands, workers, game runtimes, and extensions.

This minimizes initial files but creates weak compile-time boundaries, a large privilege and crash radius, and unavoidable resident cost for optional functionality. It also encourages managed child processes to share the daemon's lifetime. Rejected.

### Resident microservices for every subsystem

Authentication, metrics, jobs, games, backup, extensions, and storage could each run as a service and communicate over a network protocol.

This creates operational and protocol overhead that is disproportionate for a local single-host control plane. It increases base memory and makes availability depend on a web of resident processes. Remote nodes do not justify distributing the local foundation. Rejected.

### Small resident control plane with on-demand isolation

Keep one required unprivileged daemon, factor portable responsibilities into crates, and create separate operating-system processes only when privilege, resource use, untrusted code, or workload lifetime requires it. Optional processes use local typed protocols and systemd activation or transient units. Chosen.

## Decision

### Required process

`helixd` is the only continuously resident Helix process required in the base installation. It owns transport, authentication, authorization, orchestration, state coordination, lightweight demand-aware observations, events, jobs, and static frontend serving.

It runs as a dedicated unprivileged account. It never becomes an arbitrary command runner and never directly loads untrusted native plugins.

### On-demand client

`helixctl` is an administrative client for status, setup, diagnostics, recovery, and explicit offline repair. Normal commands use the same domain policies and supported interfaces as the API. Offline repair must announce when it requires a stopped daemon or elevated filesystem access and must preserve evidence before destructive action.

### Planned isolated processes

- `helix-privd` is a minimal root broker, preferably systemd socket activated. It exposes a versioned allowlist of typed operations and independently validates the kernel-authenticated peer, target, paths, arguments, expected state, and limits. It has no TCP server, frontend, plugin host, archive parser, or free-form command operation.
- `helix-worker` is a one-shot worker for expensive or failure-prone jobs. systemd scopes or transient services apply memory, CPU, I/O, runtime, and cancellation policy. Workers receive a narrow job capability, not broad daemon authority.
- `helix-strandd` is an optional extension host. A Wasm runtime or similar sandbox is not linked into `helixd` and consumes no resident memory when no installed Strand needs it.

These process crates are introduced when their phase begins, not as empty proof of modularity.

### Managed workloads

Game servers and adopted services are independent systemd units with stable ID-derived names. They are not child processes whose lifetime depends on `helixd`. Helix observes systemd as runtime authority and reconciles state after reconnecting. Unit relationships must not cause a daemon stop, upgrade, or uninstall to stop a managed workload unless that action was explicitly requested.

### Foundation crate graph

The initial workspace contains:

- `helix-core`: domain identifiers, invariants, state machines, shared errors, and interfaces;
- `helix-auth`: canonical identities, password policy/hashing, and opaque-token primitives without HTTP or database coupling;
- `helix-config`: typed process configuration, validation, defaults, and filesystem path policy;
- `helix-state`: critical SQLite schema, migrations, repositories, transactions, and integrity operations;
- `helix-secrets`: portable authenticated secret envelopes and redacted master-key credentials over `helix-state`;
- `helix-system`: narrow portable read-only host discovery and metric snapshots, with Linux target validation still required;
- `helix-api`: versioned HTTP contracts, routes, middleware, extraction, and response mapping;
- `helixd`: dependency wiring, task supervision, process lifecycle, and static asset delivery;
- `helixctl`: CLI parsing and administrative presentation.

The later `helix-strand-kit` library contains only non-executing preview
scaffolding and bounded manifest validation used by `helixctl`. It is not the
planned `helix-strandd` process, does not start the extension-runtime phase, and
must not gain package installation, host calls, or sandbox responsibilities.

Dependencies flow inward toward `helix-core` and remain acyclic. Domain invariants do not live only in Axum handlers, SQL triggers, or frontend code.

`helix-system` is deliberately read-only. It does not manage services, packages, firewalls, users, mounts, or cgroups. It returns requested typed snapshots and does not start a permanent polling loop when constructed. Mutations that require authority cross the future broker boundary.

### Boundary protocols

Local cross-process protocols are:

- versioned and length bounded;
- encoded as closed request and response types;
- authenticated using operating-system peer identity where applicable;
- correlation-ID aware and safely auditable;
- subject to timeouts, cancellation, and output limits;
- incompatible by default when a version or operation is unknown.

A successful caller-side validation does not remove the receiver's duty to validate.

## CIA consequences

### Confidentiality

The unprivileged daemon and each optional process receive only the credentials and filesystem access they need. The broker does not return raw secret-bearing output. Workers use job-scoped capability material. Strands receive mediated handles instead of bulk secret exports. Separate processes still share a kernel and cannot protect against root or a fully compromised host.

### Integrity

Typed domain and IPC operations prevent caller-selected shell programs, unit names, and paths from becoming privileged actions. Stable identifiers map to trusted targets. Jobs record intent and expected revisions before cross-resource side effects. Compile-time crate boundaries reduce accidental coupling but do not replace runtime authorization or validation.

### Availability

Managed workloads survive control-plane failure. Metrics, workers, and Strands have separate failure paths. Socket activation and one-shot work avoid permanent base cost. More process boundaries add startup, packaging, IPC compatibility, and diagnosis work, which must be tested rather than assumed reliable.

## Consequences

Positive:

- small base residency without putting root or untrusted runtimes in `helixd`;
- independent game-process lifetime;
- clearer testing and ownership;
- optional heavy work can use cgroup limits and crash independently;
- portable domain code can be tested without systemd.

Costs:

- typed IPC protocols and compatibility policy require design and tests;
- package units, service identities, sockets, and permissions are more involved;
- job and operation reconciliation is required across crashes;
- some data structures need transport and domain representations;
- crate boundaries can become ceremony if responsibilities are split too finely.

The response to excessive boundary overhead is to combine adjacent library responsibilities after measurement, not to collapse privileged or workload-lifetime isolation.

## Validation

Before these boundaries can be considered tested:

- restart and upgrade `helixd` while a managed fixture and real game workloads continue running;
- crash and resource-exhaust workers and the Strand host;
- verify disabled optional components create no processes, listeners, timers, or runtime linkage;
- fuzz and compatibility-test local protocols;
- test broker peer credentials, authorization-independent validation, path races, timeouts, and output bounds;
- measure base RSS, startup, and binary size with optional features absent;
- confirm `helix-system` construction and idle dashboard state do not cause uncontrolled polling.

## Revisit triggers

Revisit this ADR if measured IPC cost prevents a required workflow, if an operating-system boundary cannot enforce the assumed isolation, if remote-node support requires a distinct agent process, or if a crate repeatedly changes for unrelated reasons. Remote operation may add a node agent and mutually authenticated protocol; it does not turn the local daemon into a distributed consensus system.
