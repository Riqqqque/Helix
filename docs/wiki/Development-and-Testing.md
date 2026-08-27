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

The frontend gate covers lint, component/adapter tests, production build, and
compressed initial-asset budgets. Rust tests cover portable behavior plus pure
or mocked Linux boundaries where a real host mutation would be unsafe.

A Windows or mocked pass does not prove Linux filesystem races, systemd, Docker,
UFW, APT, AMP, reboot, or real Minecraft lifecycle behavior. Run the exact
Linux checks relevant to a change on an isolated target, record whether each
operation was real or mocked, and preserve unrelated workloads.

Never run package, firewall, reboot, storage stress, or destructive server tests
against a host with irreplaceable data or active users. A capacity claim also
needs the exact game/version/hardware/configuration/load evidence; a synthetic
fixture proves only control-plane bounds.

## Preview Strand projects

```text
helixctl strand new system-health --name "System Health"
helixctl strand check system-health
```

These commands create and validate metadata without installing or executing an
extension. See
[Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands).

More detail:

- [Development](https://github.com/Riqqqque/Helix/blob/main/docs/DEVELOPMENT.md)
- [Contributing](https://github.com/Riqqqque/Helix/blob/main/CONTRIBUTING.md)
- [Performance](https://github.com/Riqqqque/Helix/blob/main/docs/PERFORMANCE.md)
- [Progress](https://github.com/Riqqqque/Helix/blob/main/PROGRESS.md)
