# Contributing to Helix

Helix is early-stage systems software. Small, well-tested changes are more useful than broad feature scaffolding, and a green build does not by itself make a runtime, game, backup, or security claim true.

External code contributions are paused until the repository owner selects and
documents project and contribution licensing terms. Bug reports, design
discussion, and review feedback remain welcome.

## Before starting

- Read [the architecture](docs/ARCHITECTURE.md), [security model](docs/SECURITY.md), and [roadmap](ROADMAP.md).
- Read [the support guide](SUPPORT.md) before opening a question or defect.
- Check `PROGRESS.md` for what has actually been verified and `NEXT.md` for the current resumption point.
- Open an issue before introducing a new privileged boundary, database, runtime dependency, plugin model, wire format, or large frontend framework.
- Never put credentials, recovery material, private host data, game data, or proprietary assets in an issue, fixture, log, or commit.

## Development approach

Keep changes focused and preserve the dependency direction described in the architecture. Optional features must not create idle work when disabled. Validate input at its trust boundary, bound memory and concurrency, and keep important writes recoverable.

Game integrations require current upstream evidence. Verify the exact game version, server distribution, loader, mappings, Java or compatibility-runtime requirement, API, licensing, and download source before changing an integration. A mock process is not proof that a game works.

See [Development](docs/DEVELOPMENT.md) for repository conventions and platform limits.

## Required checks

From the repository root:

```text
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
cargo build --locked --release --workspace --all-targets --all-features
```

From `frontend`:

```text
npm ci --no-audit --no-fund
npm run check
npm audit --audit-level=moderate
```

Run the Linux, migration, recovery, packaging, security, and performance checks relevant to the change. If an environment is unavailable, say exactly what could not be tested; do not replace it with a weaker claim.

Shell packaging changes also require:

```text
bash -n scripts/package-common.sh scripts/install-local.sh scripts/rollback-local.sh scripts/uninstall-local.sh scripts/build-release.sh scripts/test-linux-package.sh
shellcheck -x scripts/package-common.sh scripts/install-local.sh scripts/rollback-local.sh scripts/uninstall-local.sh scripts/build-release.sh scripts/test-linux-package.sh
```

## Pull requests

Describe:

- the behavior and reason for the change;
- the exact commands run and their results;
- tests and failure cases added;
- migration, permission, privacy, recovery, and rollback effects;
- measured binary, bundle, idle, or latency impact when relevant;
- remaining risks and platform-specific validation still needed.

Documentation, changelog entries, commit messages, and UI copy should be direct and practical. Avoid marketing claims that the evidence does not support.

## Security reports

Do not open a public issue for a vulnerability that could put users or data at risk. Follow [SECURITY.md](SECURITY.md) instead.
