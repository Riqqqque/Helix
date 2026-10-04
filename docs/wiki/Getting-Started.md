# Getting Started

Helix is built for a headless Linux server: you install it over SSH with one
command, then use it from a browser on any other computer on your network.

> [!CAUTION]
> Helix is a private-LAN release. Keep it on a network you control, do not
> forward its dashboard port to the internet, and keep independent backups of
> anything important.

## What you need

- A 64-bit Linux server (x86_64 or ARM64) with **systemd**: Ubuntu, Debian,
  Fedora, Rocky/Alma, openSUSE, Arch, and similar. The installer is tested on
  fresh Ubuntu 22.04 and 24.04 machines.
- **Docker Engine with the Compose plugin** (`docker compose`). If it is
  missing, the installer can install it for you with Docker's official script.
- A private network address on the server, such as `192.168.1.50`.
- About 10 GB of free disk and 4 GB of RAM for the first build. Game servers
  need their own memory on top of that.
- `curl`, which almost every server already has.

You do not need Rust, Node.js, or a desktop on the server. Everything is built
inside Docker.

## Install

SSH into the server and run:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash
```

The installer:

1. Checks the server: Linux, systemd, a 64-bit CPU, and the tools it needs.
2. Checks Docker and offers to install it if it is missing.
3. Downloads the latest Helix release and verifies its SHA-256 checksum.
4. Finds the server's private address and network (for example
   `192.168.1.50` on `192.168.1.0/24`) and asks you to confirm them.
5. Creates the `helix-broker` and `helix-terminal` system groups and the folders
   under `/srv/helix` for game servers, backups, and Helix's own data.
6. Writes the broker settings to `/etc/helix/privd.json` and the deployment
   settings to `/opt/helix/helix.env`, both readable only by root.
7. Builds the Helix containers and the host broker. The first build takes
   several minutes; later builds reuse most of the work.
8. Installs and starts the `helix-privd` service, then the dashboard and gateway
   containers.
9. If UFW is active, offers to allow your network to reach the dashboard port.
10. Checks the dashboard responds, then prints where to go:

```text
Helix is running.

  Open http://192.168.1.50:3100 from any computer on 192.168.1.0/24.
  One-time owner setup token: Zm9vYmFy…
  Paste it within 15 minutes, then create your login.
```

Open that address on your laptop, paste the token, and choose a login, a
password, and the name Helix should greet you with. That's it — head to
**Servers → New server** to create your first game server.

## Installer options

Pass options after `bash -s --`, for example:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash -s -- --storage /mnt/media
```

| Option | What it does |
| --- | --- |
| `--yes` | Accept the defaults and answer yes, for scripts and provisioning tools |
| `--lan-ip ADDRESS` | Use this private address instead of the detected one |
| `--cidr NETWORK/BITS` | The network allowed to open Helix (default: the detected subnet) |
| `--port PORT` | Dashboard port (default 3100) |
| `--storage DIR` | A folder Helix may browse and manage, such as a media drive; repeatable |
| `--terminal-user USER` | Turn on the in-browser terminal for an existing non-root login |
| `--install-docker` | Install Docker without asking if it is missing |
| `--open-firewall` | Add the UFW rule without asking when UFW is active |
| `--version vX.Y.Z` | Install a specific release |
| `--reconfigure` | Rewrite the generated settings, for example after the server's address changes |
| `--new-token` | Issue a fresh owner token if the first one expired before you used it |
| `--token-file FILE` | Write the owner token to a file instead of printing it |

A fully unattended install, for cloud-init, Ansible, or a provisioning script:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh \
  | sudo bash -s -- --yes --install-docker --open-firewall --token-file /root/helix-token
```

## Check, repair, update, and remove

The installer stays on the server at
`/opt/helix/releases/<version>/scripts/install.sh`. You can also run the
one-line command again at any time.

- **Health check:** `sudo bash /opt/helix/releases/<version>/scripts/install.sh --check`
  reports whether Docker, the broker, the containers, and the dashboard address
  are healthy, with the broker's last log lines when something is wrong.
- **Repair:** running the installer again fixes missing pieces and keeps your
  settings, data, and game servers.
- **Update:** use **Host → Update Helix** in the dashboard. It downloads the
  SHA-256-pinned release, rebuilds Helix, replaces the broker, health-checks,
  and rolls back on failure. Game servers keep running.
- **Uninstall:** `--uninstall` stops and removes Helix's containers, services,
  and binaries, and keeps `/srv/helix` (game servers, worlds, backups) and your
  settings. `--uninstall --purge` also deletes those after you type `DELETE`.

## Using Helix from outside your home network

Helix only listens on your private network. To reach it from elsewhere, join
the server and your devices to a private network such as Tailscale (Helix's
**Hooks** page can install Tailscale on Debian and Ubuntu), then reinstall with
`--reconfigure --lan-ip <the server's Tailscale 100.x address>`. Never forward
the dashboard port on your router.

A server with only a public address (most cloud VPS plans) has no private
network for Helix to use. The installer stops and explains this instead of
exposing the dashboard to the internet. Add the VPS to Tailscale first, then
install with `--lan-ip` set to its Tailscale address.

## Make the dashboard yours

After the first owner exists:

- type your city on the Home weather widget;
- use the scratchpad note for ISP details, port forwards, or weekend plans, not
  passwords;
- open **Edit layout** to add shortcuts and drag widgets wherever you like;
- create your first game server from **Servers → New server**, or bring one over
  from AMP or Pterodactyl with **Copy an existing server**;
- pick a theme in Settings, and use **Arrange** to hide, add, or reorder pages.

Never post setup tokens, passwords, cookies, private addresses, hostnames,
storage paths, server logs, or world data in an issue or screenshot.

## Other ways to run Helix

- **Manual deployment.** Every step the installer automates is documented in the
  [container deployment guide](https://github.com/Riqqqque/Helix/blob/main/docs/CONTAINER-DEPLOYMENT.md),
  for operators who want to review or customise each piece.
- **Dashboard only.** `./scripts/install-from-source.sh` from a clone installs
  just the web service on loopback, without game servers or host controls.
- **Loopback preview for development** on Windows, macOS, or Linux:

  ```bash
  (cd frontend && npm ci --no-audit --no-fund && npm run build)
  cargo build --locked --release --workspace
  mkdir -p .helix-data/development
  ./target/release/helixctl --data-dir "$(pwd)/.helix-data/development" setup-token
  ./target/release/helixd --listen 127.0.0.1:8080 \
    --data-dir "$(pwd)/.helix-data/development" --web-root "$(pwd)/frontend/dist"
  ```

  This needs Rust 1.88+ and Node.js 22.12+, and host controls stay unavailable
  without the Linux broker.
