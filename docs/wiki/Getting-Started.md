# Getting Started

## Before you run Helix

Helix is a private alpha with no supported binary release. Evaluate it on a
private network with backups and a clear rollback path. Do not forward its port
to the public internet or use live data as a first test.

The repository is licensed under `AGPL-3.0-or-later`; that does not turn this
source workflow into a supported distribution.

Building the complete source requires Rust 1.88 or newer, Node.js, npm, and a
current browser. Full host and native-server controls additionally require a
reviewed Linux broker configuration and Docker on the Linux host.

## Build the source

```text
cd frontend
npm ci --no-audit --no-fund
npm run check
cd ..

cargo build --locked --release --workspace --all-features
```

## Run a loopback preview

The loopback preview is useful for setup, authentication, Home, and read-only
dashboard work. Broker-backed controls report unavailable until
`helix-privd` is configured.

From the repository root in a Bash shell:

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

Open the loopback URL printed by the daemon, use the one-time token, and create
the owner. The token expires after 15 minutes and is invalidated when replaced
or consumed.

## Private-LAN deployment

The repository contains a constrained dashboard/gateway Compose example and a
separate systemd example for the root broker. They are deployment building
blocks, not a one-command supported installer. Every bind address, Host,
Origin, client CIDR, broker group, socket, and managed storage root must match
the target host before anything starts.

Read the
[container deployment guide](https://github.com/Riqqqque/Helix/blob/main/docs/CONTAINER-DEPLOYMENT.md)
and the
[current security model](https://github.com/Riqqqque/Helix/blob/main/docs/SECURITY.md)
first. The optional Hooks installer can install and start the exact Tailscale
package on an eligible Debian/Ubuntu host, but the owner must authenticate it
and explicitly configure the secondary private gateway. Helix does not open
router ports.

Never post setup tokens, passwords, cookies, CSRF proofs, private addresses,
hostnames, storage paths, server logs, or world data in an issue or screenshot.
