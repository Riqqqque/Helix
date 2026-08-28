# Getting Started

## Before you run Helix

Helix is a private alpha with no supported binary release. Evaluate it on a
private network with backups and a clear rollback path. Do not forward its
dashboard port to the public internet or use live data as a first test.

The repository is licensed under `AGPL-3.0-or-later`; that does not turn this
source workflow into a supported distribution.

Building the complete source requires Rust 1.88 or newer, Node.js 22.12 or
newer, and npm. Full host and native-server controls additionally require a
reviewed Linux broker configuration and Docker on the Linux host.

## Choose a path

1. **Loopback preview** on the machine you cloned. This is enough to create the
   owner, see Home greet your name, and use read-only dashboard pages.
   Broker-backed host, storage, network, and native-server controls stay
   unavailable until `helix-privd` is configured on Linux.
2. **Private LAN on a Linux server you control.** Copy the examples, replace
   every placeholder with that host's address, groups, and storage roots, then
   follow the container deployment guide. This is not a one-command installer.

The local package flow in
[Installation](https://github.com/Riqqqque/Helix/blob/main/docs/INSTALLATION.md)
is a scoped lifecycle test. For a first server, use Compose plus the broker
unit rather than that package.

## Build the source

From the repository root:

```text
cd frontend
npm ci --no-audit --no-fund
npm run build
cd ..

cargo build --locked --release --workspace --all-features
```

`npm run check` and the workspace test/clippy gates are the contributor
verification path. They are not required just to produce `frontend/dist` and
the release binaries.

## Run a loopback preview

This path works on Windows, macOS, and Linux. Host mutations still need the
Linux broker.

From the repository root in a Bash shell (Git Bash or WSL is fine on Windows):

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

Open the loopback URL printed by the daemon. Paste the one-time token and
create the owner with a login you will remember and a **display name**. Home
greets that name. The token expires after 15 minutes and is invalidated when
replaced or consumed.

Do not put the token or password in `.env`, shell history, logs, screenshots,
or source.

## Put it on a Linux server

The repository contains a constrained dashboard/gateway Compose example and a
separate systemd example for the root broker. Every bind address, Host, Origin,
client CIDR, broker group, socket, and managed storage root must match the
target host before anything starts.

1. Copy `.env.example` to `.env`. Replace `192.168.1.10` and `192.168.1.0/24`
   with this server's private IPv4, the origin you will type in a browser, and
   the subnet those browsers sit on. Set the two distinct broker/terminal GIDs
   and the persistent data/backup directories. Wildcards are not allowed.
2. Copy `deploy/privd.example.json` to `/etc/helix/privd.json`. Replace every
   `/srv/...` path with directories you created on this host. Do not copy the
   example roots blindly. `/` as an analysis root is for largest-file scans, not
   a custom-JAR import boundary.
3. Keep `ReadWritePaths` in `deploy/helix-privd.service` aligned with those
   same directories.
4. Follow the
   [container deployment guide](https://github.com/Riqqqque/Helix/blob/main/docs/CONTAINER-DEPLOYMENT.md)
   and the
   [current security model](https://github.com/Riqqqque/Helix/blob/main/docs/SECURITY.md).

The optional Hooks installer can install and start the exact Tailscale package
on an eligible Debian/Ubuntu host, but the owner must authenticate it and
explicitly configure the secondary private gateway. Helix does not open router
ports for the dashboard.

## Make the dashboard yours

After the first owner exists:

- type your city on the Home weather widget;
- use the scratchpad note for ISP details, port forwards, or weekend plans, not
  passwords;
- open **Edit layout** to add shortcuts and rearrange widgets;
- create the first native Minecraft server from **Servers → New server** (Helix
  Native stays separate from any AMP import);
- pick a theme and navigation order in Settings.

Never post setup tokens, passwords, cookies, CSRF proofs, private addresses,
hostnames, storage paths, server logs, or world data in an issue or screenshot.
