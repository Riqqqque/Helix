# Development and Testing

## Portable validation

```text
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
cargo build --locked --release --workspace --all-features

cd frontend
npm ci --no-audit --no-fund
npm run check
npm audit --audit-level=moderate
```

Shell changes also run Bash syntax checks and ShellCheck. The checked-in CI
workflow is configured to build with the declared Rust 1.88 minimum and stable
Rust, scan dependencies and secrets, and exercise the package lifecycle on
Ubuntu 24.04 with systemd. No hosted run has passed yet, so that workflow is
configuration, not Ubuntu validation evidence.

A green portable suite does not prove systemd, Unix permissions, low-disk,
power-loss, restore, game, or performance behavior. Claims must name the exact
platform and test that supports them.

See [Development](https://github.com/Riqqqque/Helix/blob/main/docs/DEVELOPMENT.md),
[Contributing](https://github.com/Riqqqque/Helix/blob/main/CONTRIBUTING.md), and
[Performance](https://github.com/Riqqqque/Helix/blob/main/docs/PERFORMANCE.md).
