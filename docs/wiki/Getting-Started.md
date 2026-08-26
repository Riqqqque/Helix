# Getting Started

## Before you run Helix

Helix is an alpha preview. Keep it on loopback, use disposable data, and do not
place it in front of a public reverse proxy. There is no supported installer or
binary release yet; this page builds a development preview from source.

You need Rust 1.88 or newer, Node.js 22.12 or newer, npm, and a current browser.

These commands are maintainer/developer notes for evaluating the current source.
They do not grant a license or create a supported distribution path.

## Build

```text
cd frontend
npm ci --no-audit --no-fund
npm run build
cd ..

cargo build --locked --release --workspace --all-features
```

## Run an isolated preview

From the repository root in a Bash shell (this does not imply support for that
host platform):

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

Open `http://127.0.0.1:8080`, paste the one-time token, and create the owner.
The token expires after 15 minutes and is invalidated when it is replaced or
consumed.

On later runs with the same data directory, start `helixd` directly;
`setup-token` is only for an unclaimed installation. Press `Ctrl+C` in the daemon
terminal for a clean source-preview shutdown.

Never post the token, a session cookie, a CSRF proof, a password, or private host
data in a GitHub issue or screenshot.

See the repository's
[Installation model](https://github.com/Riqqqque/Helix/blob/main/docs/INSTALLATION.md)
before using the Linux package tooling.
