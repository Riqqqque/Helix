# Helix Progress

Last updated: 2026-08-26

This file records verified implementation state. [ROADMAP.md](ROADMAP.md)
describes ordering and intent; it is not evidence that a feature exists.

Status vocabulary: **NOT STARTED**, **DESIGNING**, **IMPLEMENTING**,
**IMPLEMENTED — UNVALIDATED**, **TESTED**, **BLOCKED**, **COMPLETE**.

## Overall status

Helix is a founding pre-release application. The portable Phase 0 and secure-core
slice are real and testable. Ubuntu 24.04 GitHub Actions passed the declared Rust
1.88/stable builds and one scoped systemd package lifecycle, but Phase 0 remains
**BLOCKED** on clean supported-host, cross-version upgrade, broader permission and
recovery-fault, restore, signing, and reference-performance matrices. Helix is not
ready for production, public-network exposure, or a packaged public release.

No game integration, backup restore, privileged host mutation, update channel,
Genome import/export, or Strand runtime is supported. A portable encrypted
secret-record boundary exists, but production master-key delivery, rotation, and
independent recovery do not.

## Phase 0 — Foundation

| Area | Status | Current evidence and limits |
| --- | --- | --- |
| Repository and architecture | TESTED | Architecture, storage, security, recovery, Genome, Sequence, Strand, backup, API, installation, development, roadmap, ADR, and public-repository documents exist and were reviewed against the founding directive. Legal license selection remains open. |
| Rust workspace | TESTED | Nine founding crates compile on Windows and hosted Ubuntu with locked dependencies. Formatting, Clippy warnings-as-errors, tests, release builds, Rust stable, and the declared Rust 1.88 MSRV pass. Clean supported-host and additional architecture validation remain open. |
| Configuration | TESTED | Strict typed TOML, explicit overrides, loopback-only policy, absolute roots, and canonical disjoint state/static roots have adversarial coverage. One packaged Ubuntu run exercised the installed configuration path; broader ownership and replacement-race matrices remain target-host gates. |
| Critical state | TESTED | SQLite WAL/FULL policy, schema versions 1–4, stable installation/node identity, integrity and semantic checks, unclean marker, online snapshots, process lease, private-file checks, and read-only CLI paths have portable tests. Migration retries reuse only a fully verified source/version/target/SHA-bound rollback snapshot; changed sources get a new snapshot, altered aliases fail closed, and at most 16 legacy partials are reconciled per open. One scoped Ubuntu package run passed full doctor, verified database-only backup, and selected ownership/mode checks; complete Unix race and fault matrices remain open. |
| Replaceable metrics | TESTED | Separate WAL/NORMAL database, corruption preservation/recreation, and degradation without critical-state loss have portable tests. Published forensic-set count/retention, disk-full, read-only mount, and sustained-retention policy/tests remain; persistent metric writers are not enabled yet. |
| Daemon lifecycle | TESTED | Loopback listener, structured logging, API/static composition, exclusive writer lease, recovery-before-cleanup ordering, tracked blocking-task drain, and clean-marker ordering have focused tests. A live Windows run covered forced-stop recovery and clean Ctrl+C. One scoped Ubuntu package run verified systemd identity/hardening, forced-crash restart, and clean stop/start; broader signal, watchdog, and injected-fault matrices remain open. |
| CLI | TESTED | Read-only status/doctor, verified state-only online backup, absolute-deadline readiness, and lease-protected one-time setup-token creation are implemented and passed a release-binary flow. Repair and restore commands do not exist yet. |
| HTTP foundation | TESTED | Versioned API, JSON API fallback, SPA fallback, restrictive headers, request IDs, compression, body/timeout limits, strict loopback Host validation, global concurrency bounds, and single-flight host sampling have service tests on Windows and hosted Ubuntu. Network-socket flood and broader supported-host matrices remain. |
| Host overview | TESTED | Real hostname, OS, architecture, kernel, uptime, CPU, memory, swap, storage, and cumulative network counters are collected on demand. Windows live data rendered two mounts and two interfaces; a scoped Ubuntu package run returned an authenticated overview with a logical CPU count and explicit storage/network availability states. An unlabeled volume is represented as an explicit text omission rather than invalid empty text. Supported-host correctness and reference-performance matrices remain. |
| Frontend | TESTED | The compiled Preact/Vite UI uses real protected APIs, honest degraded states, visibility-aware polling, semantic native meters, explicit transition focus, visible forced-colors focus, reduced-motion support, responsive layouts, and System/Midnight/OLED/Light themes. Live Windows browser checks passed login, reload-to-login CSRF behavior, sign-out, desktop OLED, and a 320-pixel layout with no horizontal overflow or console errors. Formal screen-reader and representative-device review remain. |
| CI and dependency policy | TESTED | Actions and tool versions are pinned. Ubuntu 24.04 GitHub Actions passed Rust stable, the declared Rust 1.88 MSRV, frontend checks, pinned Rust advisory/license/source policy, current/full-history secret scans, CodeQL, and the scoped package lifecycle for commit `fdbbc0a`. This does not establish clean-host or platform support. |
| Packaging and installer | TESTED | Hardened systemd/sysusers/tmpfiles fixtures, checksum-producing Linux bundles, transactional package-file install/rollback, and conservative data-preserving uninstall exist. One scoped Ubuntu 24.04 lifecycle passed archive verification, fresh install, owner claim, protected API/authentication flows, selected file modes and unit hardening, forced-crash restart, clean stop/start, full doctor, verified backup, modified-bundle manifest rejection, repeat install, explicit rollback, and data-preserving uninstall. Public package support remains blocked; artifacts are unsigned and must not be attached to a release. |
| Performance baseline | BLOCKED | A non-reference Windows development snapshot is recorded in [docs/PERFORMANCE.md](docs/PERFORMANCE.md). Required Ubuntu RSS, ten-minute idle CPU, repeated startup, API/SQLite latency, and installed-size measurements remain unavailable. |

## Phase 1 — Secure core, Lattice, and Pulse

| Area | Status | Current evidence and limits |
| --- | --- | --- |
| Password/token primitives | TESTED | Argon2id v19 hashing/verification, bounded PHC parsing, parameter upgrades, prospective-password validation, canonical identities, fallible CSPRNG use, and domain-separated 256-bit opaque tokens have focused portable tests. The policy basis is documented; this is not a NIST-conformance claim. Supported-Ubuntu parameter benchmarking, maintained compromised-password data, and independent review remain. |
| Owner bootstrap and state schema | TESTED | Schema v2 implements a race-safe single-use bootstrap, owner/role/capability/session/audit state, progressive login delay, expiry/revocation, auth-version invalidation, and atomic verified-password rehash. Login cleanup is bounded to 256 rows per attempt, converges normal state to at most 64 sessions per user, returns a generic retryable 503 while oversized imports remain, and keeps eviction/rehash/new-session/audit changes atomic. |
| Browser login/session/CSRF | TESTED | The loopback setup, owner claim, login, protected reads, CSRF compare-and-swap rotation, logout/revocation, Host/Origin/Fetch-Metadata checks, cookie flags, generic failures, and reload-to-login rule passed unit, API, live compiled-asset, and one packaged Ubuntu flow. TLS, trusted proxies, MFA, broader Linux authentication matrices, brute-force field testing, and independent review remain gates. |
| Portable recoverable-secret records | TESTED | Schema v3 and `helix-secrets` implement XChaCha20-Poly1305 record envelopes, fresh DEKs/nonces, master-key wrapping/check records, associated-data binding, CAS revisions, bounded plaintext, and zeroizing/redacted access. Daemon credential delivery, systemd credential lifetime, rotation/rewrapping, fallback key files, TPM use, and independent recovery are not implemented. |
| Chronicle authentication audit | TESTED | Authentication/session events reject secret-bearing detail and remain append-only inside their retained window. Schema v4 retains the newest 1,024 rows, applies a 90-day window outside that floor, targets at most 50,000 rows, and removes at most 256 rows per audited write or startup pass with older denials first under record pressure. Fixed-batch convergence, counter/trigger semantics, rollback boundaries, and schema tamper rejection have portable tests. Export, holds, hash chaining, tamper evidence, off-host forwarding, and broader operator events remain future work. |
| Lattice layouts and Pulse history/events | NOT STARTED | The current authenticated overview is a real responsive foundation. Persistent layouts, historical metrics, adaptive retention, and a versioned event/reconnect stream do not exist. |

## Validation snapshot

The final local pass used Rust 1.94.1 (`x86_64-pc-windows-msvc`), Cargo
1.94.1, Node 24.12.0, and npm 11.6.2 on Windows 11 x64. The declared MSRV pass
used Rust 1.88.0 on the same target.

- `cargo fmt --all -- --check`
- `cargo check --locked --workspace --all-targets --all-features`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace --all-targets --all-features`
- `cargo build --locked --release --workspace --all-features`
- `cargo doc --locked --workspace --all-features --no-deps`
- `cargo +1.88.0 check --locked --workspace --all-targets --all-features`
- `cargo +1.88.0 test --locked --workspace --all-targets --all-features`
- `npm run check` and `npm audit --audit-level=moderate` in `frontend`

The settled suite contains 152 target-conditioned Rust tests on Windows, 158 on
hosted Ubuntu, and 48 frontend tests. The final fresh Windows release-binary flow
passed setup, owner claim, login, protected reads, CSRF
rotation, cookie-only rejection, logout/revocation, generic login failure,
progressive delay, CLI ready/status/full-doctor/backup, 1,500 latency requests,
real storage/network collection, and a ten-file canary scan with zero secret
leaks.

Current release artifacts measured 7,564,800 bytes for `helixd.exe`, 2,853,376
bytes for `helixctl.exe`, and 10,418,176 bytes combined. The compiled frontend
measured 91,782 bytes raw, 24,796 bytes gzip, and 21,872 bytes Brotli. These
Windows values are non-reference.

For commit [`fdbbc0a`](https://github.com/Riqqqque/Helix/commit/fdbbc0aeb7b353069c74fe5186d1d48fd65b66ca),
Ubuntu 24.04 GitHub Actions passed Rust 1.88/stable, the 158-test
target-conditioned suite, the frontend gate, the scoped systemd package
lifecycle, and CodeQL. Pinned `cargo-audit` 0.22.2 scanned 196 locked dependencies
against 1,226 loaded RustSec advisories with no finding. Pinned `cargo-deny`
0.20.2 passed advisories, bans, licenses, and sources with six reviewed
duplicate-version warnings. npm reported zero vulnerabilities. Pinned Gitleaks
8.30.1 found no leak in either the committed checkout or full Git history.

CodeQL's initial 36 path alerts were reviewed individually: 33 were narrow false
positives on trusted operator-root boundaries guarded by strict path validation
in production code, and three were test-only fixtures. Each dismissal records
its rationale, the queries remain enabled, and the later analysis completed with
zero open alerts.

The Windows measurements remain non-reference. The hosted result establishes
provenance only for its exact revision and scope; it is not clean-VM,
cross-version-upgrade, schema-downgrade, complete fault-injection,
low-disk/power-loss, reference-performance, platform-support, signed-release, or
supported-installer evidence.

## Packaged release gate

**BLOCKED.** Required clean supported-host and cross-version package matrices,
independent authentication review, production master-key lifecycle, broader
audit-event coverage, export, holds, tamper evidence, off-host forwarding,
complete permissions/filesystem tests, restore, corruption/disk-full/power-loss
drills, signed update integrity, formal screen-reader review, representative
mobile review, game lifecycle matrices, and reference performance have not
passed. No signed binary/package release or deployment recommendation is
authorized by the current evidence.
