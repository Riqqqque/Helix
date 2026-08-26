# Development

## Project state

Helix is in its founding implementation phase. The repository may compile while core security, recovery, packaging, and runtime behavior remain incomplete. A green build is necessary evidence, not a release claim.

Use `PROGRESS.md` for verified feature state and `NEXT.md` for the exact resumption point. [Roadmap](../ROADMAP.md) describes ordering rather than completion.

## Supported development environments

Portable Rust and frontend work can be performed on Windows, macOS, or Linux. The product target is Linux with systemd and cgroup v2, so these areas require a Linux environment:

- `/proc` and `/sys` host discovery;
- systemd units, D-Bus, sockets, transient services, and watchdogs;
- cgroup resource controls;
- Unix ownership, modes, symlinks, and local peer credentials;
- package installation, upgrade, rollback, and uninstall;
- low-disk, power-loss, and process-lifetime tests;
- reference CPU, RSS, startup, and I/O measurements.

Do not replace a missing Linux test with a permissive mock and call the behavior validated. Portable mocks are useful for domain tests; real Linux tests remain a separate gate.

## Toolchain

The workspace currently declares:

- Rust edition 2024;
- minimum Rust version 1.88;
- Node.js 22.12 or newer for frontend development only;
- npm scripts in `frontend/package.json`.

The committed Cargo and frontend lockfiles, once present, are the dependency source of truth. Use the project-declared versions rather than adding global tools or silently upgrading dependencies. Node.js is never a production runtime dependency.

Useful optional local tools include `cargo-audit` and `cargo-deny`. CI should run them once their policy files are committed. Do not install system-wide tools on another contributor's machine without permission.

## Workspace layout

| Path | Responsibility |
| --- | --- |
| `crates/helix-core` | Domain types, invariants, state machines, and interfaces |
| `crates/helix-auth` | Canonical identities, password policy and hashing, and opaque-token primitives |
| `crates/helix-config` | Typed process configuration and path policy |
| `crates/helix-state` | Critical SQLite state, repositories, migrations, and integrity operations |
| `crates/helix-secrets` | Portable authenticated-encryption and master-key boundary for recoverable secrets |
| `crates/helix-system` | Narrow read-only host discovery and metrics adapters |
| `crates/helix-api` | Versioned HTTP contracts, routing, and middleware |
| `crates/helixd` | Daemon composition and lifecycle |
| `crates/helixctl` | Administrative CLI |
| `frontend` | Preact, TypeScript, tests, and static asset build |
| `docs` | Architecture and operational contracts |

Keep dependency flow toward `helix-core`. API handlers should not contain raw SQL or Linux parsing. `helix-system` must not gain privileged mutations. Composition roots wire implementations; they should not become dumping grounds for reusable behavior.

## First checkout

From the repository root:

```powershell
cargo check --locked --workspace --all-targets --all-features

Set-Location frontend
npm ci
npm run check
npm audit --audit-level=moderate
```

The committed `package-lock.json` is required for clean and CI builds. Use `npm ci` rather than rewriting it incidentally. An intentional dependency change updates `package.json` and the lockfile together and records its audit, bundle, and licensing impact.

Return to the repository root before running workspace commands:

```powershell
Set-Location ..
```

## Canonical validation

Run the full portable Rust suite:

```powershell
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo build --locked --release --workspace --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
```

Run the frontend suite:

```powershell
Set-Location frontend
npm ci
npm run check
npm audit --audit-level=moderate
```

`npm run check` includes linting, tests, TypeScript checking, the production
build, and compressed-size reporting. That report is evidence to record, not by
itself proof that interaction or runtime budgets pass.

On Linux, add package and system tests appropriate to the change. Changes to service management, paths, process supervision, storage, permissions, backup, or installation are incomplete without them.

## Running in development

Development mode must use an explicit disposable data root and non-production port. Never point a development binary at `/var/lib/helix` or a real game instance by accident.

Build the frontend first, then use resolved absolute data and web roots. On
Linux or macOS:

```bash
mkdir -p .helix-data/development

./target/release/helixctl \
  --data-dir "$(pwd)/.helix-data/development" \
  setup-token

./target/release/helixd \
  --listen 127.0.0.1:8080 \
  --data-dir "$(pwd)/.helix-data/development" \
  --web-root "$(pwd)/frontend/dist"
```

The command-line source of truth remains:

```powershell
cargo run -p helixd -- --help
```

The frontend development server can render the static shell from `frontend`:

```powershell
npm run dev
```

The checked-in Vite configuration intentionally has no API proxy. Authenticated
flows require exact Host and Origin handling, so `npm run dev` is currently for
static-shell work only. Run `npm run build` and serve `frontend/dist` through
`helixd` for production-like setup, login, session, and host-data testing. Do
not weaken the daemon's production request boundary for development convenience.

## Rust conventions

- Keep unsafe Rust forbidden unless a future ADR establishes a narrowly reviewed exception.
- Model identifiers, revisions, capabilities, sizes, and paths with domain types instead of interchangeable strings.
- Validate at trust boundaries and preserve invariants inside the domain.
- Avoid blocking filesystem, process, or SQLite work on Tokio executor threads.
- Bound channels, buffers, subprocess output, concurrency, and retries.
- Use structured errors with stable categories; keep sensitive internal context out of API responses.
- Prefer readable safe code over speculative allocator, pointer, or micro-optimization work.
- Do not add a background task without defining activation, cadence, cancellation, shutdown, failure, and disabled-state cost.
- Do not introduce a dependency without checking maintenance, licensing, security history, default features, binary impact, and whether the standard library is enough.

Warnings fail Clippy in the canonical suite. Formatting is enforced by rustfmt. Public behavior needs tests; internal modules do not need decorative abstractions or excessive comments.

## Frontend conventions

- Server resources remain authoritative; do not turn local storage into a hidden database.
- Keep the initial route free of chart, terminal, editor, game-specific, and Strand code.
- Lazy-load expensive routes and libraries.
- Use semantic HTML, keyboard-operable controls, visible focus, sufficient contrast, and reduced-motion support.
- Treat loading, empty, stale, disconnected, permission-denied, partial, and failed states as designed states.
- Avoid large component frameworks, icon packs, web fonts, and runtime CSS systems without measured justification.
- Keep API types explicit. Validate untrusted responses when a mismatch could become unsafe.
- Never persist bearer tokens, recovery keys, or secrets in local storage.
- Tests must assert user-observable behavior rather than component internals where practical.

The framework decision and reversal gate are documented in [ADR 0003](adr/0003-frontend-framework.md).

## Database development

Critical state and metrics are separate durability domains. [ADR 0002](adr/0002-sqlite-durability-domains.md) is normative.

For critical-state migrations:

1. add a new monotonically ordered migration; never rewrite a released migration;
2. make it transactional where SQLite permits;
3. define upgrade validation and rollback/recovery behavior;
4. update repository and integration tests;
5. test against a copy of the previous schema;
6. verify foreign keys and an integrity check;
7. ensure production migration orchestration creates a consistent pre-migration snapshot.

Tests may use temporary databases. Tests for WAL behavior, online backup, corruption response, busy handling, checkpoints, disk pressure, and unclean shutdown need real filesystem-backed databases.

Do not put game archives, worlds, logs, or arbitrary JSON ownership blobs into `helix-state.db`.

## Host adapters

`helix-system` reads host state and returns typed snapshots. Parsers must handle missing, malformed, overflowing, renamed, or permission-denied inputs without panicking. Sampling is requested by an orchestrator; constructing an adapter must not start polling.

Platform-specific code is isolated behind interfaces that can be unit tested with fixtures. Fixture success does not replace verification against supported kernels and distributions. Host mutation belongs behind the future privileged broker, never in `helix-system`.

## API development

New routes follow [API](API.md):

1. define transport types separately from persistent records;
2. authenticate and authorize;
3. validate sizes, formats, revisions, and state transitions;
4. call a domain operation;
5. map known outcomes to stable response and problem codes;
6. redact logs and responses;
7. test success, invalid input, unauthenticated, unauthorized, conflict, and failure behavior.

Do not ship fake metrics or successful no-op controls to complete a screen. A route that exposes an unfinished backend should report an explicit unavailable state or remain absent.

## Testing strategy

Use the smallest test that proves the behavior, then cover boundary failures:

- unit tests for parsers, validation, permissions, state machines, policies, and path logic;
- integration tests for SQLite, filesystem operations, APIs, jobs, authentication, and adapters;
- end-to-end tests for setup, login, install, configuration, backup, restore, upgrade, and uninstall;
- Linux system tests for systemd, cgroups, permissions, crash recovery, disk-full behavior, and managed-process independence;
- game test-lab runs for every claimed game/version/runtime combination;
- fuzzing for archives, paths, manifests, protocol frames, URLs, and imports;
- fault injection for process death during writes, migration, install, backup, restore, and update.

Keep unfavorable and inconclusive results. A mocked lifecycle test does not justify a “supported game” label.

## Performance work

Measure before optimizing. Record conditions, hardware, build profile, sample duration, tool, and raw values in `docs/PERFORMANCE.md`. Reference measurements run on a clean supported Ubuntu host and include:

- steady and peak RSS;
- idle CPU over at least ten minutes;
- warm and cold startup;
- binary and installed size;
- initial and route-level frontend assets, raw and compressed;
- API and SQLite latency distributions;
- adaptive metrics overhead.

Windows numbers may aid regression investigation but must be labeled non-reference. Do not introduce a custom allocator, disable durability, or remove validation for a headline benchmark.

## Documentation and progress

Update architecture or an ADR when changing a major boundary. ADRs are append-only decisions: supersede an accepted ADR with a new one rather than editing history to pretend the old choice never existed.

`PROGRESS.md` uses only these states:

- NOT STARTED
- DESIGNING
- IMPLEMENTING
- IMPLEMENTED — UNVALIDATED
- TESTED
- BLOCKED
- COMPLETE

State evidence should link to tests, measurements, or exact validation notes. A frontend screen is not feature completion.

When stopping mid-task, update `NEXT.md` with current state, failing commands, uncommitted work, the next highest-priority task, reproduction steps, and pending decisions.

## Secrets and local data

Never commit:

- credentials, API tokens, private keys, recovery material, or MFA seeds;
- `.env` files containing real values;
- user databases, worlds, backups, or logs;
- support bundles with machine or user data;
- generated game files or proprietary third-party assets.

Tests use synthetic fixtures and obvious non-secret credentials. Logs must redact authorization headers, cookies, tokens, secret settings, command-line secrets, and sensitive environment values.

## Pull-request evidence

A change should report:

- what behavior changed and why;
- commands run and their exact result;
- platform and relevant versions;
- tests added or updated;
- performance or bundle impact where relevant;
- migrations, operational effects, and rollback;
- remaining limitations or validation that could not be performed.

Do not describe a feature as secure, lightweight, recoverable, or supported without the corresponding evidence.
