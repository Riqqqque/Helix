# Changelog

This file records user-visible and operator-visible changes. Helix has no
binary or package release yet.

## Unreleased

### Added

- Founding nine-crate Rust workspace with typed configuration, separate
  critical-state and replaceable-metrics SQLite domains, a versioned HTTP API,
  and a local administration CLI.
- Race-safe local owner enrollment, Argon2id login, bounded and revocable
  server-side sessions, capability checks, session-bound CSRF proofs, and
  append-only authentication audit events.
- Portable encrypted secret records using per-record data keys and a separately
  supplied installation master key.
- Real read-only CPU, memory, uptime, swap, storage, and cumulative network
  discovery.
- Compiled Preact dashboard with setup/login/session-expired flows, honest
  partial states, native progress meters, responsive layouts, and System,
  Midnight, OLED, and Light themes.
- Recovery, storage, security, architecture, API, packaging, and extension
  contracts.
- Hardened systemd, local bundle, checksum-verifying installer, rollback
  scaffolding, and pinned CI/security tooling. One scoped Ubuntu 24.04 systemd
  package lifecycle passed for commit `fdbbc0a`.

### Changed

- Migration rollback snapshots are content-addressed and tied to the exact
  source schema, target schema, and source SHA-256. Identical retries reuse the
  verified snapshot; changed sources get a new one; altered aliases fail closed.
- Startup reconciles legacy migration partials in bounded batches and validates
  recovery before session cleanup.
- Shutdown drains HTTP requests and detached blocking state/password work before
  writing the clean-shutdown marker.
- Unlabeled or invalid storage text is reported as an explicit omission instead
  of breaking the host overview.
- The 320-pixel dashboard now wraps long interface names and avoids document
  overflow on browsers with non-overlay scrollbars.
- Setup values now survive validation and recoverable request failures; a real
  already-claimed conflict still clears the one-time credentials.
- Dashboard transitions move keyboard focus deliberately, health changes use a
  polite live region, forced-colors focus remains visible, and route context
  follows the active section.
- Updated `toml` to 1.1.4 while retaining Rust 1.88 compatibility and removing
  the duplicate `winnow` 0.7 dependency.

### Security

- Runtime binding remains restricted to loopback even though local
  authentication is implemented. Remote exposure still requires the TLS/proxy,
  cookie, MFA, rate-limit, and independent-review boundary.
- Protected reads and mutations require both the session cookie and the current
  in-memory CSRF proof; cookie-only requests cannot recover a new proof.
- Login cleanup is bounded, oversized imported session sets converge through a
  generic retryable response, and final eviction/rehash/session/audit changes
  commit atomically.
- Authentication/session audit retention protects the newest 1,024 events,
  applies a 90-day window beyond that floor, targets at most 50,000 rows, and
  prunes at most 256 rows per transaction.
- API responses use restrictive browser headers and separate unknown API routes
  from the frontend fallback.
- Critical state uses durable SQLite settings, verified online snapshots,
  restrictive Unix-mode policy, and explicit unclean-shutdown checks.

### Known limitations

- Helix is not ready for production deployment or public-network exposure.
- Production master-key delivery/rotation/recovery, broader audit-event coverage,
  export, holds, tamper evidence, off-host forwarding, backup restore, privileged
  host administration, historical metrics, persistent layouts, and game
  execution are not complete.
- A scoped Ubuntu 24.04 install/rollback/uninstall lifecycle has passed, but
  clean-VM, cross-version upgrade, complete permission/fault/recovery, signing,
  and reference-performance matrices remain open.
