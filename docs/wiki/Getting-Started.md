# Getting Started

Helix is one private web dashboard for a Linux server and the game servers on
it. You install it over SSH with one command, then use it from a browser on any
other computer on your network. Game servers run in their own Docker containers
and keep running when the dashboard is closed or restarted.

> [!CAUTION]
> Helix is a private-LAN release. Keep it on a network you control, never
> forward its dashboard port on your router, and keep independent backups of
> anything important.

## What you need

| Requirement | Details |
| --- | --- |
| Operating system | 64-bit Linux (x86_64 or ARM64) with **systemd** as the init system |
| Tested on | Fresh Ubuntu 22.04 and 24.04 machines, in the [Installer workflow](https://github.com/Riqqqque/Helix/blob/main/.github/workflows/installer.yml) |
| Docker | Docker Engine with the Compose v2 plugin (`docker compose`). The installer can add it for you |
| Network | A private IPv4 address on the server: `10.x`, `172.16-31.x`, `192.168.x`, or a Tailscale `100.64-127.x` address |
| Resources | About 10 GB of free disk and 4 GB of RAM for the first build, plus memory for your game servers |
| Tools | `curl`, `tar`, `sha256sum`, `ip`, and other standard tools that almost every server already has |

You do not need Rust, Node.js, or a desktop on the server. Everything is built
inside Docker.

The installer itself only checks for Linux, systemd, and a 64-bit CPU, so other
systemd distributions may install. They are not tested. Features built on APT,
such as **Linux updates** and the one-click Hooks, need Debian or Ubuntu.

## Install

SSH into the server and run:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash
```

The installer:

1. Checks the server: root, Linux, systemd, a 64-bit CPU, and the tools it needs.
2. Checks Docker. If Docker or the Compose plugin is missing, it asks whether to
   install Docker with Docker's official script.
3. Downloads the latest Helix release and checks it against the published
   SHA-256 checksum.
4. Finds the server's private address and subnet and asks you to confirm them.
5. Creates the `helix-broker` and `helix-terminal` system groups and the folders
   under `/srv/helix`.
6. Writes its settings to `/etc/helix/privd.json` and `/opt/helix/helix.env`,
   both readable only by root.
7. Builds the dashboard, the gateway, and the host broker. The first build takes
   several minutes.
8. Starts the `helix-privd` service, then the dashboard and gateway containers.
9. If UFW is active, asks whether to allow your network to reach the dashboard
   port.
10. Checks that the dashboard answers, then prints where to go.

### What it asks

On a normal SSH session you see up to four questions. Press Enter to accept the
value in brackets.

| Question | When |
| --- | --- |
| Install Docker now with Docker's official install script (get.docker.com)? | Docker or `docker compose` is missing |
| Address other computers will use to open Helix | Always on a first install |
| Network allowed to open Helix | Always on a first install |
| UFW is active. Allow your network to reach Helix on port 3100? | UFW is active and has no matching rule |

The port is not asked. It is 3100 unless you pass `--port`. To skip every
question, pass `--yes` and the options you want. See
[Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration)
for every option.

### What it prints

```text
Helix is running.

  Open http://192.0.2.10:3100 from any computer on 192.0.2.0/24.
  One-time owner setup token: <token>
  Paste it within 15 minutes, then create your login.

  Game servers:  /srv/helix/instances
  Settings:      /opt/helix/helix.env and /etc/helix/privd.json
  Updates:       Host → Update Helix in the dashboard
  Diagnose:      sudo bash /opt/helix/releases/<version>/scripts/install.sh --check
```

If the server has only a public address, as many cloud VPS plans do, the
installer stops instead of exposing the dashboard. See
[Using Helix away from home](#using-helix-away-from-home).

## Create the owner account

1. On your laptop, open the address the installer printed, for example
   `http://192.0.2.10:3100`. Use the IP address exactly as printed. A hostname
   or `localhost` is refused.
2. Paste the one-time owner setup token.
3. Choose a login name, a display name (Home greets you with it), and a
   password.

The token works once and expires after 15 minutes. If it expires before you use
it, issue a new one from the server:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash -s -- --new-token
```

`--new-token` works only until the owner account exists. After that, setup
tokens are disabled for good. There is no password reset, so store the owner
password in a password manager.

## Create your first server

Open **Servers → New server**, pick a game, choose a name and memory, and
create it. Helix downloads the server software, starts the container, and waits
until the game is ready. Each game has its own page:
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace),
[Hytale](https://github.com/Riqqqque/Helix/wiki/Hytale),
[Pumpkin](https://github.com/Riqqqque/Helix/wiki/Pumpkin), and
[Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim).

Already run a server in AMP or Pterodactyl? Use **Copy an existing server**. See
[Server Migration](https://github.com/Riqqqque/Helix/wiki/Server-Migration).

For players outside your home, forward the **game** port on your router, never
the dashboard port. The server's Overview shows the exact port and protocol.

## Using Helix away from home

Helix only listens on a private address. To reach it from elsewhere, put the
server and your devices on a private network such as Tailscale. Helix's
**Hooks** page can install Tailscale on Debian and Ubuntu; you still sign in to
your tailnet yourself. Then point Helix at the server's Tailscale address:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh \
  | sudo bash -s -- --reconfigure --lan-ip 100.64.0.10 --cidr 100.64.0.20/32
```

Replace `100.64.0.10` with the server's real Tailscale address and
`100.64.0.20/32` with the narrowest range that covers the devices you open
Helix from. The installer sets one client range, and a `/32` allows exactly
one device. This moves Helix from its LAN address to the Tailscale address; to
keep both, set up the
[secondary entry point](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration#secondary-entry-point)
instead. `--reconfigure` rewrites both settings
files, so pass any `--storage` folders again. On a VPS with only a public address,
join it to Tailscale first, then install with `--lan-ip` set to its Tailscale
address.

## Next steps

- Make Home yours: weather city, notes, shortcuts, layouts, and themes in
  [Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home).
- Set up [Backups](https://github.com/Riqqqque/Helix/wiki/Backups) and copy
  important worlds off this machine too.
- Learn the host pages in
  [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates).
- Read [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery)
  before you rely on Helix for anything important.
- If something goes wrong, start with
  [Troubleshooting and FAQ](https://github.com/Riqqqque/Helix/wiki/Troubleshooting-and-FAQ).

Never post setup tokens, passwords, cookies, private addresses, hostnames,
storage paths, server logs, or world data in an issue or screenshot.

## Other ways to run Helix

- **Manual deployment.** Every step the installer automates is described in
  [Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration)
  and the
  [container deployment guide](https://github.com/Riqqqque/Helix/blob/main/docs/CONTAINER-DEPLOYMENT.md).
- **Dashboard only.** `./scripts/install-from-source.sh` from a clone installs
  only the web service on loopback, without game servers or host controls.
- **Development preview** on Windows, macOS, or Linux: see
  [Development and Testing](https://github.com/Riqqqque/Helix/wiki/Development-and-Testing).
  Host controls stay unavailable without the Linux broker.
