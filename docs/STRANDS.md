# Strand Extension Security and Runtime

## Status

Strands are a design only. No Strand manifest, registry, host API, Wasm runtime,
sidecar protocol, capability store, UI isolation, signature verifier, or
resource limiter has been implemented. The base daemon must not gain a Wasm
runtime dependency until a real Strand requires it and measurements justify the
selected host.

A Strand extends Helix through a versioned, capability-limited boundary. It is
not a Rust dynamic library loaded into `helixd`, a route to arbitrary root, or a
claim that third-party native code is sandboxed.

## Process architecture

`helixd` contains only the small Strand control-plane interface and package
metadata needed to report installed/disabled extensions. Third-party execution
occurs in optional `helix-strandd` or a one-shot worker. When no Strand requiring
the runtime is enabled, the runtime is not resident and ideally not installed.

The host process has its own systemd service, service identity, cgroup, writable
paths, address-family policy, capability bounding, `NoNewPrivileges`, and
restart limits. A Strand crash or host crash degrades that Strand and records a
health event; it cannot terminate `helixd` or independent game services.

The `helixd`/host protocol is versioned, authenticated over a restrictive local
socket, length-bounded, cancellation-aware, and subject to queue backpressure.
The host receives capability-scoped object handles, not a reference to the
entire Helix database or filesystem.

## Strand classes

### Portable Strand

The preferred third-party form is a WebAssembly component using a pinned,
reviewed WASI/component model runtime in `helix-strandd`. The exact runtime and
interface version require an ADR and benchmarks for binary size, idle memory,
startup, fuel/epoch interruption, filesystem preopens, network mediation, and
security maintenance.

Wasm is an isolation layer, not proof of safety. Runtime vulnerabilities,
host-call bugs, excessive resource use, and granted capabilities remain risks.

### Trusted sidecar

A native sidecar is separately installed, launched, and supervised. It may be
appropriate for hardware integration or an established external service, but
native code with broad host access is high trust. The UI labels it clearly,
shows its systemd sandbox and capabilities, and never calls it sandboxed merely
because it uses a separate process.

Native sidecars are not allowed to use the privileged broker's peer identity
directly. They request typed work through `helixd`, which reauthorizes the
extension, user, object, and operation.

### UI-only Strand

A UI-only package still receives no implicit access to core browser state or
APIs. It runs inside the same isolated Strand UI boundary and declares the exact
read-only host data it needs.

## Package manifest

Each package has a stable UUID, publisher, version, manifest schema, supported
Helix/API range, content digest, origin/signature evidence, runtime class, entry
points, UI assets, configuration schema, state migration version, and requested
capabilities.

The manifest also declares:

- maximum memory, CPU/fuel, execution time, concurrent calls, queue depth, log
  rate, stored bytes, outbound bytes, and open handles;
- required host API functions and event subscriptions;
- filesystem datasets and storage quota;
- network origins/protocols and redirect requirements;
- secret purposes without naming stored secret values;
- background/schedule triggers and minimum cadence;
- UI routes, resource hashes, CSP requirements, and message schema; and
- update/migration/rollback behavior.

Unknown required fields, host calls, or capabilities reject installation. Every
collection and payload has a strict size limit. Canonical bytes for signature
verification are defined by the eventual format ADR and fixture-tested across
implementations.

## Capability model

The default is no filesystem, network, secret, privileged, instance, terminal,
process, service, backup, user, dashboard, event, or background execution
access. Capabilities are named, scoped, and revocable, for example:

- read a bounded metrics view for selected local node IDs;
- read or append to the Strand's own versioned key/value namespace;
- read a selected instance's public status, not its files or console;
- write only within the Strand's data directory under a quota;
- subscribe to selected redacted event types at a bounded rate;
- fetch HTTPS from enumerated origins through Helix's SSRF-safe client;
- use a named secret for one destination through a scoped operation;
- register a namespaced dashboard widget or command; and
- request a typed privileged operation that the host API specifically exposes.

A grant records package ID/version range, capability parameters, approving
actor, timestamp, and target scope. Installing, enabling, or updating displays
the human-readable effect and concrete objects/origins. New or broadened
capabilities require fresh approval; accepting a package update does not imply
accepting new permissions.

Host calls validate the Strand's current grant on every invocation. Cached
handles expire or are revoked when the Strand is disabled, updated, crashes,
loses permission, or the underlying user/object policy changes.

## Filesystem and persistent state

Each Strand receives a stable data directory and optional cache/temp areas by
opaque handle. It never receives `/`, `/var/lib/helix`, another Strand's data,
the state database, the key directory, or the privileged socket as a preopen.
Paths are descriptor-relative with the same traversal, symlink, mount, archive,
and atomic-write protections as core storage.

Persistent state is namespaced, quota-limited, and versioned. A Strand cannot
place arbitrary opaque JSON into core tables without an owning schema/version.
State migrations run in a staged transaction or operation-ledger workflow and
retain the previous package/state version for rollback.

Deleting a Strand separates disable, package removal, cache removal, and
persistent-data deletion. User data is retained by default until an authorized
deletion preview confirms exact paths and backup implications.

## Network

Portable Strands do not receive an ambient network socket API by default.
Outbound access goes through a Helix host call that enforces allowed schemes,
origins, redirects, DNS/IP/port policy, response/body limits, timeout,
concurrency, and byte quota. Loopback, link-local, private, multicast,
cloud-metadata, Unix-socket, and broker endpoints are denied unless a narrow
administrator grant explicitly requires a private service.

Inbound listeners are not part of the initial capability set. A future inbound
route is namespaced behind `helixd`, inherits authentication/security headers
and request limits, and cannot shadow a core route.

## Secrets

Strands never query the secret store or enumerate secret metadata globally. A
grant names one purpose and target scope. Prefer host-mediated use—for example,
send an authenticated webhook through Helix—so plaintext never enters the
Strand. If a protocol absolutely requires plaintext delivery, it uses a
short-lived descriptor/credential, explicit high-risk permission, redacted
logging, and prompt cleanup.

Configuration UIs receive set/unset/last-validated metadata, not stored secret
values. Reveal is not a generic Strand capability. A Strand's logs, errors,
events, metrics, storage, and support data are tested with canary secrets.

## Resource and availability controls

Per-Strand limits include:

- Wasm memory/table/stack limits and fuel or epoch deadlines;
- host-call timeout, response size, and concurrent-call limit;
- cgroup CPU weight/quota, memory high/max, task and file-descriptor limits;
- event queue depth with coalescing/drop policy;
- log rate and retained-byte limit;
- persistent/cache bytes and item count;
- outbound request count, duration, and bytes; and
- background cadence with jitter and disabled-by-default continuous polling.

The host applies backpressure rather than allocating unbounded queues. Repeated
timeouts, traps, protocol violations, excessive resource use, or crashes open a
circuit breaker. Helix disables the Strand, preserves diagnostic evidence under
bounds, and leaves unrelated functions operational. Restart limits prevent a
Strand or host crash loop.

## Events and automation

Events are typed, versioned, redacted, and scoped. Subscriptions declare the
minimum fields and rate. A Strand cannot subscribe to raw authentication,
secret, terminal, or file content events through a broad wildcard.

Event-triggered actions run with the intersection of the Strand grant, the
automation owner's current authority, and the target policy. Revoking the user
or Strand grant affects queued work. Feedback loops are bounded by correlation
IDs, recursion depth, rate limits, and circuit breakers.

## UI isolation

Strand UI is lazy-loaded only when used and rendered in a sandboxed iframe or an
equivalently strong isolated origin selected by an ADR. It does not execute as
arbitrary same-origin JavaScript in the core application.

The boundary provides:

- hashed immutable assets and no inline/eval requirement by default;
- a Strand-specific CSP, restricted frame capabilities, and no top navigation;
- a versioned, origin-checked, size-bounded message protocol;
- server-side authorization for every requested object/action;
- no access to core DOM, cookies, local/session storage, service workers,
  clipboard, downloads, camera, microphone, or popups without an explicit
  reviewed feature; and
- accessible focus, labeling, error, and theme tokens without sharing privileged
  application state.

The core CSP is not weakened to `unsafe-eval` or arbitrary script origins for a
Strand. UI removal immediately revokes its routes and message channel.

## Privileged operations

Strands never call `helix-privd` directly. A host API method represents one
narrow operation, accepts typed bounded data and stable IDs, checks both the
extension capability and current human/automation authority, previews
high-impact effects, and submits an auditable job. The root broker independently
checks peer identity and safety invariants.

No combination of permitted strings can express an arbitrary shell command,
unit name, package-manager argument, absolute path, or file descriptor. If a
new use case cannot fit a safe typed operation, it requires architecture and
threat-model review rather than a generic escape hatch.

## Installation, updates, and trust

Registry data and packages are untrusted. Installation stages the complete
package, verifies digest/signature and publisher binding, parses under limits,
scans the permission diff, checks compatibility, and presents the review before
publication. A valid signature says who produced the bytes; it is not a malware
or correctness verdict.

Updates retain the old package and state snapshot, re-run permission review,
stage/migrate offline, health-check the new instance, and roll back on failure.
Trust-root revocation disables affected updates and surfaces installed exposure
without automatically deleting user data.

Strand support labels must distinguish Helix-maintained, verified publisher,
community, local unsigned, and native high-trust packages. Unsigned local
installation, if enabled, requires owner authorization and receives no reduced
parsing or isolation.

## Audit and privacy

Install, permission grant/deny/revoke, enable/disable, update, migration,
circuit-break, secret use, privileged request, and removal are audited with
stable package/actor/target IDs. Payloads are structurally redacted. A Strand
cannot write directly to core audit tables or claim a privileged action
succeeded.

Extension telemetry is opt-in according to Helix privacy policy. A Strand
cannot bypass outbound policy by calling a native telemetry library in the
portable runtime. Native sidecar network access is disclosed as part of its
higher trust class.

## Root and sandbox limits

Helix cannot call a native sidecar with root or unrestricted host access
sandboxed. A root attacker can replace the host, package, runtime, grants,
broker, or kernel and read runtime secrets. Wasm/runtime isolation reduces
attack surface but depends on the selected runtime and correct host-call
implementation. These limits remain visible in the install review and security
documentation.

## Required verification

The Strand feature remains unavailable in production builds until tests cover:

- manifest/signature/version parsing, unknown required fields, and hostile size
  limits with an independent fixture corpus;
- permission install, denial, scope, update diff, revocation, and queued-job
  reauthorization;
- filesystem/path/archive/mount isolation and cross-Strand/core data denial;
- SSRF, redirects, DNS rebinding, private endpoints, response bombs, and network
  quota;
- secret canaries across host calls, logs, errors, state, events, UI, and support
  bundles;
- Wasm traps, infinite loops, allocation bombs, host-call floods, oversized
  frames, queue saturation, cancellation, and circuit breaking;
- UI XSS, CSP, iframe/origin/message confusion, route shadowing, and stale
  authorization;
- host and individual Strand crash/restart while games and `helixd` continue;
- state migration and package rollback after process kill/disk full; and
- proof that no Strand protocol can reach arbitrary root command execution.

The pinned runtime receives dependency and vulnerability review, fuzzing of all
host boundary decoders, Linux cgroup/systemd integration tests, and measured
base/active memory before Helix claims the feature is lightweight or sandboxed.
