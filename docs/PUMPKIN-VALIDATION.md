# Pumpkin integration validation

Validated on Linux x86-64 on September 5, 2026 against the official
`0.1.0-dev+26.2-26.45` release. The server reports Java 26.2 (protocol 776)
and Bedrock 1.26.45 (protocol 2169).

## Build gates

- `npm run check` from `frontend`: ESLint, 312 Vitest tests, three asset-budget
  tests, TypeScript, Vite, and precompression passed.
- `docker build --target linux-test .`: release build, `cargo fmt --all -- --check`,
  `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
  and `cargo test --locked --workspace --all-targets --all-features -- --include-ignored` passed.
- Docker `dashboard`, `gateway`, and `privd` targets built successfully.
- `git diff --check` passed.

The existing lazy-loaded terminal chunk still exceeds Vite's advisory chunk
size threshold; the enforced initial-page transfer budgets passed.

The full backup-rollback test changes Unix ownership and must run as root.
Ordinary test runs mark it ignored; the Docker Linux gate includes it, and both
GitHub Rust matrix jobs explicitly execute that test with elevated privileges.

## Real isolated-server checks

Used a separate broker, state directory, backup directory, and disposable server
with a 1 GiB memory cap, one CPU, and no players. Existing workloads were not
used as test fixtures.

- Versioned release discovery, SHA-256 verification, and native executable launch.
- Java status and private RCON commands; persistent console history.
- Separate Bedrock TCP signaling and UDP discovery with the expected protocol.
- Settings round-trip, change reporting, and stale-write rejection.
- Consistent backup, settings modification, restoration, and a pre-restore safety
  backup. The restored settings and Bedrock identity-key hashes matched.
- Graceful stop/start and completed world-save output.
- Already-current update detection without restarting the server.

Regression coverage also checks release URL/digest/architecture constraints,
TOML preservation and unsupported settings, and repeatable host-firewall setup
for Java TCP plus separate Bedrock TCP/UDP without touching unrelated rules.
Firewall mutation was tested with the command runner, not against a live router
or by enabling the host firewall.

## Limits

These checks do not establish complete Minecraft gameplay parity. Real Java and
Bedrock player login/gameplay, ARM64 execution, third-party native plugins,
internet/NAT connectivity, and cross-release upgrades were not tested. There was
no newer versioned release available for an actual upgrade test. Never treat
Paper/Bukkit, Fabric, Forge, or NeoForge content as natively compatible.

See [Pumpkin setup and compatibility](wiki/Pumpkin.md).
