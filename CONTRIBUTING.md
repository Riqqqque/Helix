# Contributing to Helix

Helix is private-alpha systems software. Small, well-tested changes are more
useful than broad feature scaffolding, and a green build does not by itself make
a runtime, game, backup, deployment, or security claim true.

Code, documentation, tests, design discussion, and review feedback are welcome.
By submitting a contribution, you agree to license it under
`AGPL-3.0-or-later`, the same terms as the project. You must have the right to
submit it and must identify copied or adapted work and its license.

## Before starting

- Read [how Helix works](docs/HOW-HELIX-WORKS.md), the
  [security model](docs/SECURITY.md), and the [roadmap](ROADMAP.md).
- Read [the support guide](SUPPORT.md) before opening a question or defect.
- Check `PROGRESS.md` for what has actually been verified and `NEXT.md` for the current resumption point.
- Open an issue before introducing a new privileged operation, database, runtime
  dependency, extension model, wire format, or large frontend framework.
- Never put credentials, recovery material, private host data, game data, or proprietary assets in an issue, fixture, log, or commit.

## Development approach

Keep changes focused and preserve the browser → unprivileged API → typed broker
boundary. Optional features must not create idle work when disabled. Validate
input at its trust boundary, bound memory and concurrency, and keep important
writes recoverable. Never add a general privileged shell or caller-selected
host command. The optional terminal is deliberately a separate non-root login
service and must never be folded into `helix-privd` or given its socket group.

Game integrations require current upstream evidence. Verify the exact game
version, server distribution, loader, mappings, Java or compatibility-runtime
requirement, API, licensing, and download source before changing an integration.
A mocked Docker/process adapter is not proof that a real game lifecycle works.

See [Development](docs/DEVELOPMENT.md) for repository conventions and platform limits.

Strand ideas should begin with the
[Strand author workflow](docs/STRAND-DEVELOPMENT.md). Keep the generated
manifest at zero capabilities until the design has a concrete need, explain
each requested capability in user-facing terms, and run
`helixctl strand check` in the Strand repository. UI zip install is
implemented. Portable Wasm, a native sidecar, or extra host calls still need
prior architecture and threat-model review.

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

Run the Linux, migration, recovery, packaging, broker, Docker, storage, network,
security, and performance checks relevant to the change. Firewall, reboot,
package, terminal peer/sudo, and destructive storage tests belong on an isolated
target. If an
environment is unavailable, say exactly what could not be tested; do not replace
it with a weaker claim.

Shell packaging changes also require:

```text
bash -n scripts/package-common.sh scripts/install-local.sh scripts/install-from-source.sh scripts/rollback-local.sh scripts/uninstall-local.sh scripts/build-release.sh scripts/test-linux-package.sh
shellcheck -x scripts/package-common.sh scripts/install-local.sh scripts/install-from-source.sh scripts/rollback-local.sh scripts/uninstall-local.sh scripts/build-release.sh scripts/test-linux-package.sh
```

## Pull requests

Describe:

- the behavior and reason for the change;
- the exact commands run and their results;
- tests and failure cases added;
- migration, permission, privacy, recovery, and rollback effects;
- measured binary, bundle, idle, or latency impact when relevant;
- remaining risks and platform-specific validation still needed.

Documentation, changelog entries, commit messages, and UI copy should be direct
and practical. Keep AMP separate from Helix-native servers, distinguish a
listener from outside reachability, and avoid package/self-update, player-count,
loader, modpack, or public-readiness claims the evidence does not support.

## Security reports

Do not open a public issue for a vulnerability that could put users or data at risk. Follow [SECURITY.md](SECURITY.md) instead.
