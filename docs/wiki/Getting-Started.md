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

1. **Linux systemd package** on a 64-bit x86_64 or aarch64 host you control.
   Clone the source and run `./scripts/install-from-source.sh`. On a terminal
   that one command walks through yes/no setup, compiles Helix, and installs
   `helixd` on loopback (another port if 8080 is taken). Host, file, and
   native-server controls stay unavailable until `helix-privd` is configured.
2. **Loopback preview** from a local `cargo`/`npm` build on Windows, macOS, or
   Linux. This is enough to create the owner, see Home greet your name, and use
   read-only dashboard pages.
3. **Private LAN with host and game controls.** Copy the examples, replace
   every placeholder with that host's address, groups, and storage roots, then
   follow the container deployment guide.

The Linux script is one command after clone, then yes/no questions on a
terminal. It is still an unsigned source build of a [scoped package lifecycle](https://github.com/Riqqqque/Helix/blob/main/docs/INSTALLATION.md),
not a signed or supported production installer.

## Install on Linux

From the repository root on a systemd host:

```bash
./scripts/install-from-source.sh
```

On a terminal the script asks before installing missing compiler packages,
offers rustup when Rust is missing or too old, picks another loopback port
when 8080 is busy (or you can pass `--port 8081`), and asks whether to start
helixd. Fresh installs print a one-time owner token; open the URL it prints
and paste the token. Need another token:

```bash
sudo -u helix -- helixctl --config /etc/helix/helix.toml setup-token
```

CI and pipes stay non-interactive. Pass `--yes` to skip prompts, or
`--install-deps` to install only the compiler packages. Rust 1.88+ and
Node.js 22.12+ are still required for the compile.

Debian/Ubuntu CI covers the package lifecycle. Fedora, RHEL-family, openSUSE,
Arch, and other systemd GNU/Linux distros are intended source-install targets.
The installer follows `/etc/os-release` even when it is a symlink, accepts
`pkgconf` when `pkg-config` is missing, and requires GNU coreutils plus
util-linux (BusyBox is not enough). OpenRC-only Alpine, NixOS, and Guix are
not installer targets. Selected APT updates, UFW writes, and one-click
Tailscale/Jellyfin installs remain Debian-family features.

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
- pick a theme in Settings, and use **Arrange** (or Settings → Navigation) to
  hide, add, or reorder pages. Globe starts hidden.

Never post setup tokens, passwords, cookies, CSRF proofs, private addresses,
hostnames, storage paths, server logs, or world data in an issue or screenshot.
