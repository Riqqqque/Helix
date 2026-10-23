# Development and Testing

How to build Helix from source, run it locally, run the checks CI runs, and
cut a release. The repository versions of these notes are
[docs/DEVELOPMENT.md](https://github.com/Riqqqque/Helix/blob/main/docs/DEVELOPMENT.md),
[docs/RELEASING.md](https://github.com/Riqqqque/Helix/blob/main/docs/RELEASING.md),
and [CONTRIBUTING.md](https://github.com/Riqqqque/Helix/blob/main/CONTRIBUTING.md).

## Toolchain

| Tool | Version | Where it is set |
| --- | --- | --- |
| Rust | 1.88 or newer, edition 2024 | `rust-version` in the workspace `Cargo.toml`. There is no `rust-toolchain` file. CI tests both 1.88.0 and current stable. |
| Node.js | 22.12 or newer for local work; CI uses 24.12.0 | `engines` in `frontend/package.json` |
| npm | The one bundled with your Node.js | `frontend/package-lock.json` |
| Docker | Any recent engine | Only for container builds and the Linux test image |

`Cargo.lock` and `frontend/package-lock.json` are committed. Build with
`--locked` and `npm ci` so you use exactly those versions. Node.js is a build
tool only; production hosts never need it.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/` | The twelve Rust crates. See [Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture). |
| `frontend/` | Preact + TypeScript app, Vitest tests, Vite config. |
| `deploy/` | systemd units, tmpfiles, nginx template, example broker config. |
| `scripts/` | Installer, packaging, release, and helper scripts. |
| `examples/` | Example Strands and integration clients. |
| `docs/` | Architecture, security, storage, API, ADRs, and these wiki sources in `docs/wiki/`. |

## Rust checks

Run these from the repository root. They match CI.

```bash
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
cargo build --locked --release --workspace --all-features
```

Workspace lints: `unsafe_code` is forbidden, and `dbg!`, `todo!`, and
`unimplemented!` are errors. Clippy warnings fail the build.

### Linux-only code

The broker's host modules (`files`, `native`, `network`, `packages`,
`hook_install`, `helix_update`, `hub`, and the rest) are compiled only on
Linux. On Windows or macOS the workspace still builds and the portable tests
run, but those modules and their tests are skipped.

To run the full suite from any machine, use the `linux-test` Docker target:

```bash
docker build --target linux-test .
```

It runs fmt, Clippy with `-D warnings`, every test including ignored ones, and
the server API smoke test, inside `rust:1.88-bookworm`.

A passing test on a mock is not proof of real systemd, Docker, UFW, APT,
reboot, or game behavior. Run those checks on a disposable Linux host, and
never against a host with data or players you care about.

## Frontend checks

```bash
cd frontend
npm ci --no-audit --no-fund
npm run check
npm audit --audit-level=moderate
```

| Script | Runs |
| --- | --- |
| `npm run lint` | `eslint . --max-warnings 0` |
| `npm run typecheck` | `tsc --noEmit` |
| `npm test` | `vitest run`, then the asset precompression tests |
| `npm run build` | `tsc --noEmit`, `vite build`, then gzip and Brotli precompression into `dist/` |
| `npm run check` | `lint`, then `test`, then `build` |
| `npm run dev` | Vite dev server for static shell work |

## Running locally

### Static frontend only

```bash
cd frontend
npm run dev
```

The Vite config has no API proxy on purpose. Sign-in and live data need the
exact Host and Origin checks of the real server, so use the dev server for
layout and styling work only.

A visual fixture for the game screens is at
`http://127.0.0.1:5173/game-preview.html`. Add `?state=locked`,
`?theme=light|midnight|oled`, or `?viewport=narrow`. Its sample data never
ships in a production build.

### Full dashboard against a throwaway data folder

Build the frontend, then run `helixd` on loopback with its own data folder:

```bash
cd frontend && npm run build && cd ..
cargo build --locked --release -p helixd -p helixctl
mkdir -p .helix-data/development

./target/release/helixctl --data-dir "$(pwd)/.helix-data/development" setup-token

./target/release/helixd \
  --listen 127.0.0.1:8080 \
  --data-dir "$(pwd)/.helix-data/development" \
  --web-root "$(pwd)/frontend/dist"
```

Open `http://127.0.0.1:8080` and use the printed setup token to create the
owner. Never point a development build at `/var/lib/helix` or a real server.
Host actions need a running broker; `helixd` finds its socket through the
`HELIX_PRIVD_SOCKET` environment variable (an absolute path).

`cargo run -p helixd -- --help` lists every flag.

### Other helixctl commands

| Command | Does |
| --- | --- |
| `helixctl setup-token` | Prints a one-time owner setup token valid for 15 minutes. |
| `helixctl status` | Shows schema and durability status. |
| `helixctl ready` | Waits for the daemon, state API, and compiled UI. Used by the container health check. |
| `helixctl doctor` | Validates SQLite settings and integrity (`--full` for a full check). |
| `helixctl backup-state` | Writes a verified snapshot of the state database. |
| `helixctl strand ...` | See [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands). |

## CI

| Workflow | Runs on | Checks |
| --- | --- | --- |
| `ci.yml` | Push to `main`, pull requests, weekly | Rust fmt, build, Clippy, tests on 1.88.0 and stable; Python tests for server discovery, integration clients, Valheim mod transactions, and backup rollback; shell syntax and ShellCheck; PowerShell syntax; `cargo-audit`, `cargo-deny`, and Gitleaks over the tree and full history; frontend lint, typecheck, test, build, `npm audit`; a real systemd install lifecycle of the release bundle on Ubuntu 24.04. |
| `codeql.yml` | Push and schedule | CodeQL analysis. |
| `installer.yml` | Changes to the installer, compose, Dockerfile, or `deploy/` | Fresh installs on Ubuntu 22.04 and 24.04: broker and containers up, LAN address answers and wrong hosts are refused, owner creation, re-run keeps data, health check, uninstall keeps data. Also installs the latest release with the README command. |
| `release.yml` | `v*` tags | Builds the source archive and `SHA256SUMS`, publishes the GitHub release, and attests it. |

## Coding conventions

Rust:

- Validate at trust boundaries; keep types specific (IDs, sizes, paths) rather
  than passing loose strings.
- Never block Tokio threads with file, process, or SQLite work.
- Bound every buffer, channel, retry, subprocess output, and queue.
- No background task without a defined start, cadence, cancel, and
  shutdown.
- No new privileged operation without an issue first. Never add a general
  shell or a caller-chosen command to the broker.
- API handlers do not contain raw SQL or Linux parsing.

Frontend:

- The server is the source of truth. Browser storage is only a convenience.
- Keep heavy code (charts, terminal, editor, game views, Strands) out of the
  first load; lazy-load it.
- Semantic HTML, keyboard access, visible focus, contrast, and reduced-motion
  support.
- Loading, empty, stale, denied, partial, and failed states are real states,
  not afterthoughts.
- Never put tokens or secrets in browser storage.
- Test what a user sees, not component internals.

Database:

- Add a new migration; never edit a released one.
- Test upgrades from the previous schema, not only fresh databases.

## Commits

- One focused change per commit.
- The subject is a short plain sentence in the imperative, with no prefix
  tag, for example `Update Vite, Vitest and source-map-js for their
  advisories`.
- No `Co-authored-by` trailers. Copy `scripts/git-hooks/commit-msg` to
  `.git/hooks/commit-msg` to strip them.

## Contributing

- Read [How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works),
  [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery),
  and the roadmap first.
- Open an issue before adding a privileged operation, database, runtime
  dependency, extension model, wire format, or large frontend library.
- Game integrations need current upstream evidence: exact game version,
  server build, loader, runtime, license, and download source.
- A pull request says what changed and why, the exact commands run and their
  results, tests added, migration and rollback effects, and what could not be
  tested.
- Contributions are licensed under `AGPL-3.0-or-later`.
- Report security problems privately through
  [a security advisory](https://github.com/Riqqqque/Helix/security/advisories/new),
  not a public issue.

## Releasing

Helix uses semantic versions. A `vX.Y.Z` tag on `main`, after a green CI run,
publishes `helix-source-X.Y.Z.tar.gz` and `SHA256SUMS`. The in-dashboard
**Update Helix** button uses exactly those two files.

Before tagging:

1. Start from a clean, reviewed commit with green CI and a green Installer
   run.
2. Pass the full Rust and frontend suites with locked dependencies.
3. Pass the RustSec, dependency-policy, npm, secret, and shell checks.
4. Bump the version in the workspace `Cargo.toml` and
   `frontend/package.json`, and update `CHANGELOG.md` and `PROGRESS.md`.
5. Review the tree and history for secrets and private host data.

Do not use a prerelease suffix such as `-rc1` if the updater should accept
the release; it only applies stable `vMAJOR.MINOR.PATCH` tags.

Hosts installed with `scripts/install.sh` get new broker and terminal
binaries through **Update Helix** and the `helix-finalize-update` unit. A host
deployed by hand without that unit needs the broker replaced manually:

```bash
sudo cp -a /usr/local/libexec/helix-privd /usr/local/libexec/helix-privd.previous
sudo install -m 0755 ./helix-privd /usr/local/libexec/helix-privd
sudo systemctl restart helix-privd
```

Game containers keep running while the broker restarts.

After publishing, download the asset into a clean folder, verify its
checksum, and check the release links while signed out.

## Related pages

- [Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture)
- [Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration)
- [Roadmap and Status](https://github.com/Riqqqque/Helix/wiki/Roadmap-and-Status)
