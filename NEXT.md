# Next Helix Work

Last updated: 2026-08-26

## Current resumption point

The loopback secure-core slice is implemented and locally tested. Continue with
the highest-value work that does not pretend the blocked Linux gates passed:

1. wire the installation master key through a reviewed systemd credential path,
   define effective in-memory lifetime, and implement atomic key
   rotation/rewrapping plus independent recovery;
2. add bounded Pulse history and a versioned event/reconnect stream without an
   idle polling or write cost when no dashboard is connected;
3. persist versioned Lattice layouts and widget configuration in critical state;
4. complete formal screen-reader, zoom/scaling, reduced-motion, and
   representative-device checks;
5. design Chronicle export, holds, hash chaining, and off-host forwarding
   without weakening the fixed local retention boundary;
6. rerun the full portable suite and update this file after each boundary lands.

Do not reopen remote binding as part of these items. TLS, proxy trust, cookie
policy, MFA, rate-limit field testing, and an independent authentication review
must be handled as one explicit exposure boundary.

## Exact target-host validation

On a clean current Ubuntu Server VM with systemd and cgroup v2, start with the
checked-in CI-equivalent commands:

```bash
rustup toolchain install 1.88.0 --component rustfmt,clippy
rustup toolchain install stable --component rustfmt,clippy

cargo +stable fmt --all -- --check
cargo +1.88.0 check --locked --workspace --all-targets --all-features
cargo +1.88.0 test --locked --workspace --all-targets --all-features
cargo +stable check --locked --workspace --all-targets --all-features
cargo +stable clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo +stable test --locked --workspace --all-targets --all-features
cargo +stable build --locked --release --workspace --all-features

cd frontend
npm ci --no-audit --no-fund
npm run check
npm audit --audit-level=moderate
cd ..

bash -n scripts/package-common.sh scripts/install-local.sh \
  scripts/rollback-local.sh scripts/uninstall-local.sh scripts/build-release.sh
./scripts/build-release.sh
```

Then run the behavior that this Windows workspace cannot prove:

- verify bundle checksums, install as root, and confirm `helixd` runs only as
  the dedicated `helix` account;
- assert owners and modes for configuration, binaries, assets, state, metrics,
  lock, WAL/SHM, backup, cache, runtime, and systemd credential paths;
- test clean start/stop, SIGTERM, forced crash restart, second-daemon refusal,
  bind conflict, invalid configuration, metrics unavailable/corrupt, state
  corruption, read-only storage, low disk, interrupted migration, stale lock
  artifacts, and pending snapshot reconciliation;
- exercise packaged first-owner creation, concurrent claim rejection,
  login/logout/expiry/revocation/CSRF/Host/rate-limit failures, session
  maintenance convergence, and log/database token redaction;
- test upgrade, automatic rollback, explicit rollback, uninstall with data
  preservation, and repeated installation;
- record the full reference protocol in `docs/PERFORMANCE.md`, including at
  least ten minutes of idle CPU/wakeup evidence and repeated startup samples.

Phase 0 remains **BLOCKED**, not complete, until that matrix passes.

## Current workspace state

- The recorded validation host had no usable Linux, WSL, Docker, or Podman
  target, so no Ubuntu/systemd result is implied by the portable pass.
- The validation snapshot predates the first commit and configured remote. A
  later public source commit does not turn that snapshot into release evidence.
- Preserve any uncommitted work when resuming; do not clean, reset, or replace
  it as a shortcut.
- Disposable E2E state and project-local audit tools are safe to recreate, but
  must not be committed.

## Pending owner decisions

- Select a repository license before granting redistribution rights or
  publishing a packaged release.
- Choose initial supported Ubuntu releases and architectures only after
  clean-host testing.
- Enable and verify GitHub private vulnerability reporting before inviting
  external testing.
- Choose release-signing identities and the key-compromise recovery procedure.

## Do not do next

- Do not bind remotely, trust forwarded headers, or publish the setup surface
  merely because loopback authentication tests pass.
- Do not add a privileged broker or arbitrary shell execution to accelerate host
  controls.
- Do not claim backup restore, Genome recovery, or game support before real
  end-to-end restore and lifecycle evidence.
- Do not start Minecraft integration without rechecking the exact current game,
  Java, loader, mappings, API, dependency, and download requirements from
  official sources.
- Do not commit generated build output, local databases, logs, tokens, audit
  tools, or benchmark host identifiers.
