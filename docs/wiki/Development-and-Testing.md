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
Ubuntu 24.04 with systemd. For commit
[`6868b36`](https://github.com/Riqqqque/Helix/commit/6868b36abbd1a16665565753953abbdf1dfda148),
the hosted run passed 158 target-conditioned Rust tests, the frontend and
supply-chain gates, CodeQL, and one scoped install/authentication/restart/backup/
modified-bundle-rejection/reinstall/rollback/uninstall lifecycle.

That result does not prove every systemd or Unix-permission boundary, clean-VM
variation, cross-version upgrade, low-disk, power-loss, restore, game, signing,
or reference-performance behavior. Claims must name the exact platform and test
that supports them.

See [Development](https://github.com/Riqqqque/Helix/blob/main/docs/DEVELOPMENT.md),
[Contributing](https://github.com/Riqqqque/Helix/blob/main/CONTRIBUTING.md), and
[Performance](https://github.com/Riqqqque/Helix/blob/main/docs/PERFORMANCE.md).
