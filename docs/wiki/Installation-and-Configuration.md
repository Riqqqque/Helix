# Installation and Configuration

This page is the operator reference: every installer option, where Helix puts
its files, how to check, repair, upgrade, and remove an install, and every
setting in its configuration files. For a quick first install, start with
[Getting Started](https://github.com/Riqqqque/Helix/wiki/Getting-Started).

> [!CAUTION]
> Helix is a private-LAN release. The dashboard listens on one private address
> and serves plain HTTP. Do not bind it to a public address, forward its port,
> or put it behind a public tunnel.

## The pieces

| Piece | Runs as | What it does |
| --- | --- | --- |
| `dashboard` container (`server-dashboard`) | UID `10001`, no capabilities | `helixd`: the web UI, the API, and Helix's own state |
| `gateway` container (`server-dashboard-gateway`) | nginx as UID `101`, no capabilities | The only listener on the LAN port. Checks Host, Origin, client address, method, and body size |
| `helix-privd.service` | root, on the host, in a systemd sandbox | The host broker: game servers, files, network, packages, power, and updates, through a closed set of typed requests |
| `helix-terminald@USER.service` | One ordinary Linux user | Optional browser terminal for that user |
| `helix-finalize-update.service` | root, one-shot | Finishes an in-app Helix update |
| Game server containers (`helix-game-*`) | Their own numeric users | The games. Independent of the dashboard containers |

## Requirements

- 64-bit Linux (x86_64 or aarch64) with systemd as PID 1. The installer checks
  only these. Continuous testing covers fresh Ubuntu 22.04 and 24.04.
- Root access (`sudo`).
- Docker Engine with the Compose v2 plugin, or let the installer add Docker.
- These commands: `ip`, `getent`, `groupadd`, `install`, `sed`, `awk`, `tar`,
  `sha256sum`, `timeout`, `systemctl`, and `curl`. The updater needs `curl` too.
- A private IPv4 address on the server (`10.0.0.0/8`, `172.16.0.0/12`,
  `192.168.0.0/16`, or `100.64.0.0/10`).
- About 10 GB of disk and 4 GB of RAM for the first build.

APT-based features (Linux updates, one-click Hook installs) need Debian or
Ubuntu. UFW features need UFW.

## Installer options

Run the installer with options after `bash -s --`:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh \
  | sudo bash -s -- --storage /mnt/media --open-firewall
```

### Install options

| Option | Default | What it does |
| --- | --- | --- |
| `--yes`, `-y` | off | Accept every default and answer yes. Required when no terminal is attached, for example in cloud-init |
| `--version vX.Y.Z` | latest release | Install a specific release. The `v` is optional |
| `--source DIR` | none | Install from an existing Helix source tree instead of downloading. For development |
| `--lan-ip ADDRESS` | detected | The private IPv4 address browsers use. Must be private and assigned to this server |
| `--cidr NETWORK/BITS` | detected subnet | The client network allowed to open Helix. Must be `/8` or narrower |
| `--port PORT` | `3100` | Dashboard port on the LAN address, from 1024 to 65535 |
| `--storage DIR` | none | An extra existing folder Helix may browse and manage. Repeat for more folders. `/`, `/etc`, `/usr`, `/boot`, `/proc`, `/sys`, `/dev`, and `/run` are refused |
| `--terminal-user USER` | none | Turn on the browser terminal for an existing non-root login that is not in `helix-broker` |
| `--install-docker` | ask | Install Docker with Docker's official script if it is missing, without asking |
| `--open-firewall` | ask | Add the UFW rule for the dashboard port without asking, when UFW is active |
| `--reconfigure` | off | Rewrite `/opt/helix/helix.env` and `/etc/helix/privd.json` instead of keeping them |
| `--new-token` | off | Issue a fresh owner setup token. Only useful before the owner account exists |
| `--token-file FILE` | print | Write the owner setup token to `FILE` (mode `0600`) instead of printing it |

### Other modes

| Option | What it does |
| --- | --- |
| `--check` | Diagnose an existing install and exit. Makes no changes |
| `--uninstall` | Remove the Helix services, binaries, and dashboard containers. Keeps game servers, data, and settings |
| `--purge` | With `--uninstall`, also delete Helix data, settings, game servers, worlds, and backups after you type `DELETE` |
| `--allow-downgrade` | Build this release even if a newer Helix is already installed |
| `-h`, `--help` | Show the built-in help |

### Environment variable

| Variable | Default | What it does |
| --- | --- | --- |
| `HELIX_REPO` | `Riqqqque/Helix` | The GitHub `owner/repo` the installer downloads releases from |

### Unattended install

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh \
  | sudo bash -s -- --yes --install-docker --open-firewall --token-file /root/helix-token
```

With `--yes`, the installer takes the detected address and subnet. If you know
them, pass `--lan-ip` and `--cidr` too.

## What the installer does

1. **Checks the server.** Root, Linux, systemd, CPU architecture, required
   commands.
2. **Fetches the release.** It reads the latest tag from the GitHub API (or uses
   `--version`), downloads `helix-source-<version>.tar.gz` and `SHA256SUMS`,
   checks the archive against its checksum, rejects archives with unexpected
   paths, and unpacks it to `/opt/helix/releases/<version>`. Then it runs the
   installer from inside that release, so the installer always matches the
   files it installs.
3. **Checks Docker** and starts the Docker service if it is installed but
   stopped.
4. **Finds the address.** It picks the address used for the default route. A
   public address is replaced with a private one if the server has one;
   otherwise the install stops. The subnet comes from the interface prefix; a
   prefix broader than `/16` is narrowed to `/16`.
5. **Checks the port.** If another program already listens on it, the install
   stops.
6. **Creates groups and folders:** `helix-broker`, `helix-terminal`, and the
   tree under `/srv/helix`.
7. **Writes settings** (see [Configuration reference](#configuration-reference)).
8. **Builds** the dashboard and gateway images with Docker Compose, and builds
   `helix-privd` and `helix-terminald` in Docker, then copies them to
   `/usr/local/libexec`. A changed `helix-privd` is kept as
   `helix-privd.previous`.
9. **Installs units** and starts `helix-privd`. It waits up to 30 seconds for
   the broker socket and prints the broker log if it does not appear.
10. **Issues the owner token** with `helixctl setup-token` if this is the first
    start, or if you passed `--new-token`.
11. **Starts** the dashboard and gateway and waits until both are healthy.
12. **Offers a UFW rule** allowing your client network to the dashboard port.
13. **Checks** that `http://<address>:<port>/healthz` answers, then prints the
    summary.

## Install layout

### Host paths

| Path | Mode | Contents |
| --- | --- | --- |
| `/opt/helix/helix.env` | `0600` root | Deployment settings for Compose |
| `/opt/helix/releases/<version>/` | `0755` | Verified release source the installer downloaded and built |
| `/etc/helix/privd.json` | `0600` root | Broker configuration |
| `/etc/helix/amp.json` | `0600` root | Optional AMP credentials |
| `/usr/local/libexec/helix-privd` | `0755` | Broker binary (`helix-privd.previous` is the one before it) |
| `/usr/local/libexec/helix-terminald` | `0755` | Terminal bridge binary |
| `/etc/systemd/system/helix-privd.service` | `0644` | Broker unit |
| `/etc/systemd/system/helix-privd.service.d/storage.conf` | `0644` | Extra `ReadWritePaths` for `--storage` folders |
| `/etc/systemd/system/helix-finalize-update.service` | `0644` | In-app update finalizer |
| `/etc/systemd/system/helix-terminald@.service` | `0644` | Terminal template unit |
| `/etc/tmpfiles.d/helix.conf` | `0644` | Creates `/run/helix`, `/run/helix/terminal`, `/run/helix/hook-installs`, `/var/lib/helix/updates`, `/var/lib/helix/docker-cleanup` |
| `/run/helix/privd.sock` | `0660` root:helix-broker | Broker socket |
| `/run/helix/terminal/terminal.sock` | `0660` user:helix-terminal | Terminal socket, when enabled |
| `/srv/helix/instances/` | `0750` | Game server folders |
| `/srv/helix/backups/` | `0750` | Game server backups and the backup trash |
| `/srv/helix/state/instances/` | `0700` | Server definitions, job records, console history, staged file changes, removed servers, imports |
| `/srv/helix/state/network/` | `0700` | Helix-owned firewall rule records |
| `/srv/helix/dashboard/data/` | `0700` UID 10001 | Dashboard state: accounts, sessions, preferences, tokens, audit, metrics |
| `/srv/helix/dashboard/backups/` | `0700` UID 10001 | `helixctl backup-state` snapshots |
| `/var/lib/helix/` | `0700` root | Broker state: updates, Docker cleanup schedule, recurring reboot records |
| `/var/lib/helix-hub/` | root | Machine hub SSH keys |

Inside the dashboard container, `/srv/helix/dashboard/data` is mounted at
`/var/lib/helix`, `/srv/helix/dashboard/backups` at `/var/lib/helix-backups`,
and `/run/helix` read-only at `/run/helix-host`. The host's `/var/lib/helix`
and the container's `/var/lib/helix` are different folders.

### systemd units

| Unit | Type | Notes |
| --- | --- | --- |
| `helix-privd.service` | simple, enabled | `User=root`, `Group=helix-broker`, `Restart=on-failure`. Sandboxed with `ProtectSystem=strict`, `ProtectHome=read-only`, `NoNewPrivileges`, and explicit `ReadWritePaths` |
| `helix-finalize-update.service` | oneshot | Started by the updater. Runs `helix-privd --finalize-pending-update`, up to 15 minutes |
| `helix-terminald@USER.service` | simple | One instance per terminal user. Only accepts the dashboard's UID `10001` |
| `helix-docker-cleanup.service` and `.timer` | created on demand | Written when you save a Docker cleanup schedule |
| `helix-recurring-reboot.service` and `.timer` | created on demand | Written when you save a recurring reboot |

The broker sandbox allows writes to `/run/helix`, the Docker socket,
`/var/lib/helix`, `/srv/helix`, `/srv/storage`, `/srv/amp/instances`, `/boot`,
`/etc`, `/usr`, and `/var`. A leading `-` makes the optional paths skip cleanly
when they do not exist. The OS paths are there for APT updates, UFW, and
schedules. Removing them makes those features fail instead of half-working.

### Compose services

| Setting | `dashboard` | `gateway` |
| --- | --- | --- |
| Image | `helix-dashboard:<version>` | `helix-dashboard-gateway:<version>` |
| Root filesystem | read-only | read-only |
| Memory / CPU / processes | 128 MiB / 1.0 / 128 | 64 MiB / 0.5 / 64 |
| Network | `helix-private` bridge, inter-container traffic off | Shares the dashboard's network namespace |
| Health check | `helixctl ready` every 15 seconds | `GET /gateway-healthz` every 15 seconds |
| Restart policy | `unless-stopped` | `unless-stopped` |
| Logs | `json-file`, 3 files of 10 MB | same |

Both drop every Linux capability, set `no-new-privileges`, and carry a label
that tells Watchtower to leave them alone. The Compose project name is
`server-dashboard`.

### Ports

| Listener | Address | Notes |
| --- | --- | --- |
| Dashboard | `HELIX_LAN_BIND_ADDRESS:HELIX_LAN_PORT` (default port `3100`) | The only published Helix port. Served by the gateway on port `3000` inside the container |
| `helixd` | `127.0.0.1:8080` inside the dashboard namespace | Never published. `helixd` refuses any non-loopback listen address |
| Broker | `/run/helix/privd.sock` | Unix socket only, no TCP |
| Game servers | Per server | Published by each game container. See [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace#ports) |

The dashboard answers only on the exact LAN address with the matching Host. On
the server itself, `http://127.0.0.1:3100` does not answer; use the LAN
address.

## Check, repair, and upgrade

### Health check

```bash
sudo bash /opt/helix/releases/<version>/scripts/install.sh --check
```

`--check` reports, one line each:

- Docker is running;
- `/opt/helix/helix.env` and `/etc/helix/privd.json` exist;
- `helix-privd` is running (with its state, restart count, and last 5 log lines
  if not);
- the broker socket `/run/helix/privd.sock` exists;
- `server-dashboard` and `server-dashboard-gateway` are healthy;
- the saved address is still on this server; and
- the dashboard answers `/healthz` at the saved address and port.

It exits non-zero when it finds a problem. Any copy of the installer can run
`--check`, including the one-line command, which skips the download for
`--check` and `--uninstall`:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash -s -- --check
```

### Repair or re-run

Re-run the one-line command. It downloads the latest release, rebuilds,
reinstalls the units, and starts everything again. It keeps:

- `/opt/helix/helix.env` and `/etc/helix/privd.json`, unless you pass
  `--reconfigure`. The group IDs in `helix.env` are always refreshed;
- the saved address, client network, and port. If the saved address is no
  longer on the server, the installer stops and asks for `--reconfigure`;
- every game server, world, backup, and dashboard account.

`--reconfigure` writes both files from scratch: pass every `--storage` folder
again, and any hand edits to `privd.json` are lost. The AMP credentials line is
added back automatically when `/etc/helix/amp.json` exists.

> [!NOTE]
> After an in-app update, the folder under `/opt/helix/releases/` still holds
> the version you first installed. Since 1.10.2 the installer refuses to build a
> release older than the running dashboard unless you pass `--allow-downgrade`,
> so running an old copy cannot roll Helix back by accident. Use the one-line
> command to repair with the latest release.

### Upgrade

Use **Host → Update Helix** in the dashboard. It is covered in
[Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates#updating-helix).
Re-running the one-line installer also upgrades to the latest release.

## Uninstall

```bash
sudo bash /opt/helix/releases/<version>/scripts/install.sh --uninstall
```

It asks for confirmation (default No; `--yes` skips it), then:

- stops and removes the dashboard and gateway containers;
- disables every `helix-terminald@` instance and `helix-privd`;
- removes the Helix units, the `storage.conf` drop-in, the tmpfiles entry, and
  the binaries in `/usr/local/libexec`.

It keeps `/srv/helix` (game servers, worlds, backups, server state, dashboard
data), `/opt/helix`, `/etc/helix`, the game server containers, and the two
system groups. Re-installing picks them up again.

`--uninstall --purge` also removes every `helix-game-*` container and deletes
`/srv/helix`, `/opt/helix`, `/etc/helix`, and `/var/lib/helix`. It needs a
terminal and the typed word `DELETE`; it refuses to run with `--yes`. This
cannot be undone.

Uninstall also disables and removes the schedules you created in the dashboard
(`helix-docker-cleanup.timer` and `helix-recurring-reboot.timer`, with their
services), so they do not keep calling the removed broker. Installers before
1.10.2 left them behind; disable them with `systemctl disable --now` if you
uninstalled an older version.

## Manual and container deployment

The installer automates the private-LAN container deployment. To review or
customise each piece, follow the
[container deployment guide](https://github.com/Riqqqque/Helix/blob/main/docs/CONTAINER-DEPLOYMENT.md).
In short:

1. Build from one exact source revision.
2. Create the `helix-broker` and `helix-terminal` groups (different GIDs).
3. Write `/etc/helix/privd.json` (root, `0600`) from
   [`deploy/privd.example.json`](https://github.com/Riqqqque/Helix/blob/main/deploy/privd.example.json),
   using folders that exist on this host.
4. Install the units from
   [`deploy/`](https://github.com/Riqqqque/Helix/tree/main/deploy) and the
   broker binary, then start `helix-privd`.
5. Copy [`.env.example`](https://github.com/Riqqqque/Helix/blob/main/.env.example)
   to `.env` (`0600`) and set every value.
6. Create the data and backup folders, owned by `10001:10001`, mode `0700`.
   Compose will not create them.
7. Issue the owner token with the dashboard stopped:

   ```bash
   docker compose stop gateway dashboard
   docker compose run --rm --no-deps --entrypoint /app/bin/helixctl dashboard \
     --config /app/config/helix.toml setup-token
   docker compose up -d --wait
   ```

Do not use `--remove-orphans`, restart Docker globally, or prune shared Docker
objects as part of a Helix deployment.

A dashboard-only package also exists: `./scripts/install-from-source.sh`
compiles a checkout and installs `helixd` on loopback, with no broker, game
servers, or host controls. Its layout is described in
[`docs/INSTALLATION.md`](https://github.com/Riqqqque/Helix/blob/main/docs/INSTALLATION.md).

## Configuration reference

Every Helix configuration file rejects unknown keys. A typo stops the service
with a parse error instead of being ignored.

### Deployment settings: `helix.env`

The installer writes `/opt/helix/helix.env` and passes it to Compose with
`--env-file`. In a manual deployment the same keys live in `.env`.

| Key | Installer value | Meaning |
| --- | --- | --- |
| `COMPOSE_PROJECT_NAME` | `server-dashboard` | Compose project name |
| `HELIX_LAN_BIND_ADDRESS` | detected or `--lan-ip` | The one host address the dashboard port is published on |
| `HELIX_LAN_PORT` | `3100` or `--port` | Published dashboard port |
| `HELIX_BROKER_GID` | GID of `helix-broker` | Added to the dashboard so it can reach the broker socket |
| `HELIX_TERMINAL_GID` | GID of `helix-terminal` | Added to the dashboard so it can reach the terminal socket |
| `HELIX_GATEWAY_HOST` | same as the address | The only Host header the gateway accepts |
| `HELIX_BROWSER_ORIGIN` | `http://<address>:<port>` | The only Origin allowed to send POST, PUT, and DELETE |
| `HELIX_ALLOWED_CLIENT_CIDR` | detected or `--cidr` | Client addresses allowed through the gateway |
| `HELIX_SECONDARY_GATEWAY_HOST` | `unused.invalid` | Second accepted Host. Inert by default |
| `HELIX_SECONDARY_BROWSER_ORIGIN` | `http://unused.invalid` | Second accepted Origin. Inert by default |
| `HELIX_SECONDARY_ALLOWED_CLIENT_CIDR` | `192.0.2.254/32` | Second client range. Inert by default |
| `HELIX_DATA_DIR` | `/srv/helix/dashboard/data` | Host folder mounted as the dashboard's data folder |
| `HELIX_BACKUP_DIR` | `/srv/helix/dashboard/backups` | Host folder mounted for state snapshots |

Two more keys are read by Compose and set by the installer and updater at build
time, not stored in the file: `HELIX_IMAGE_TAG` (image tag, the version) and
`HELIX_SOURCE_REVISION` (recorded build revision).

Never use wildcard Host, Origin, or client range values.

#### Secondary entry point

The three `HELIX_SECONDARY_*` keys let the gateway accept a second exact Host,
Origin, and client range, for an already configured private route such as
Tailscale Serve. The dashboard still listens only on `HELIX_LAN_BIND_ADDRESS`.
Edit `/opt/helix/helix.env`, then re-run the installer without
`--reconfigure` so the gateway is recreated with the new values. Do not trust the whole `100.64.0.0/10` range because one device uses
Tailscale. This route becomes part of your security boundary, so review it
yourself.

### Dashboard: `helix.toml`

The dashboard image ships
[`deploy/helix-container.toml`](https://github.com/Riqqqque/Helix/blob/main/deploy/helix-container.toml)
at `/app/config/helix.toml`. The file is limited to 64 KiB.

| Key | Default | Container value | Meaning |
| --- | --- | --- | --- |
| `server.listen` | `127.0.0.1:8080` | `127.0.0.1:8080` | `helixd` listen address. Must be loopback; remote binds are refused |
| `paths.data_dir` | `/var/lib/helix` | `/var/lib/helix` | State folder: `state/helix-state.db`, `metrics/helix-metrics.db`, `secrets/` |
| `paths.web_root` | `/usr/share/helix/web` | `/app/web` | Compiled UI. Must not overlap `data_dir` |

`helixd` and `helixctl` read settings in this order: built-in defaults, the TOML
file (`--config`, else `HELIX_CONFIG`, else `/etc/helix/helix.toml` if present),
environment variables, then command-line options. These variables are unrelated
to the Compose keys of the same name in `helix.env`.

| Variable | Meaning |
| --- | --- |
| `HELIX_CONFIG` | Path to the TOML file |
| `HELIX_LISTEN` | Overrides `server.listen` |
| `HELIX_DATA_DIR` | Overrides `paths.data_dir` |
| `HELIX_WEB_ROOT` | Overrides `paths.web_root` |
| `HELIX_PRIVD_SOCKET` | Broker socket. Compose sets `/run/helix-host/privd.sock`. Without it, host actions, files, and servers are unavailable |
| `HELIX_TERMINAL_SOCKET` | Terminal socket. Compose sets `/run/helix-host/terminal/terminal.sock` |
| `RUST_LOG` | Log filter. Compose sets `helixd=info,tower_http=warn` |
| `HELIX_LOG_FORMAT` | Set to `json` for JSON logs |

Compose also sets `MALLOC_ARENA_MAX`, `MALLOC_MMAP_THRESHOLD_`, and
`MALLOC_TRIM_THRESHOLD_` to keep memory use low after password checks.

### Broker: `privd.json`

`helix-privd` reads `/etc/helix/privd.json` (or `--config PATH`). The file must
be owned by root, have no group or other permissions, be at most 64 KiB, and use
absolute paths. The installer writes:

```json
{
  "socket": "/run/helix/privd.sock",
  "managed_roots": ["/srv/helix/instances"],
  "analysis_roots": ["/"],
  "host_control": { "docker_cleanup_state_root": "/var/lib/helix/docker-cleanup" },
  "native": {
    "state_root": "/srv/helix/state/instances",
    "instance_root": "/srv/helix/instances",
    "backup_root": "/srv/helix/backups",
    "docker_binary": "/usr/bin/docker"
  },
  "network": { "state_root": "/srv/helix/state/network" }
}
```

Each `--storage` folder is appended to `managed_roots`, and
`"amp_credentials": "/etc/helix/amp.json"` is added when that file exists.
Restart the broker after an edit:

```bash
sudo systemctl restart helix-privd
```

Restarting the broker does not stop game servers.

#### Top level

| Key | Required | Default | Meaning |
| --- | --- | --- | --- |
| `socket` | yes | none | Broker socket path. Must match the Compose mount: `/run/helix/privd.sock` |
| `managed_roots` | yes | none | 1 to 32 folders that Files and Storage may browse and change. A root that no longer exists is skipped with a warning; if none exist, the broker stops. A root that is a symlink or otherwise unsafe stops the broker |
| `analysis_roots` | no | same as `managed_roots` | Up to 32 folders for read-only largest-file analysis. Never writable |
| `amp_credentials` | no | none | Path to the AMP credentials file. Omit it if you do not use AMP |
| `native` | no | none | Helix game servers. Without it, server pages report "the Helix server manager is not configured" |
| `host_control` | no | defaults below | Host, Docker, power, and schedule settings |
| `network` | no | defaults below | Network and UFW |
| `packages` | no | defaults below | APT updates |
| `hook_installer` | no | defaults below | One-click Hook installs |
| `helix_update` | no | defaults below | In-app Helix updater |
| `hub` | no | defaults below | Machine hub (SSH probes and power for other machines) |

#### `native`

| Key | Default | Meaning |
| --- | --- | --- |
| `state_root` | required | Server definitions, `jobs.json`, console history, staged file changes (`pending/`), removed servers (`server-trash/`), imported JARs |
| `instance_root` | required | Game server folders |
| `backup_root` | required | Game server backups and the backup trash |
| `docker_binary` | `/usr/bin/docker` | Docker CLI. The installer writes the path it finds |
| `console_history_max_bytes` | `536870912` (512 MiB) | Console history kept per server. 16 MiB to 32 GiB |
| `console_history_files` | `32` | Number of rotating console files per server. 2 to 256 |
| `backup_trash_retention_days` | `7` | Host default for how long deleted backups stay in the trash before Helix empties it. `0` keeps them until you delete them. At most 365. Each server can override it |
| `custom_artifact_roots` | every managed root that is a real folder below `/` | Folders a custom server JAR may be picked from. Must be absolute, not `/`, and not a symlink |

#### `host_control`

| Key | Default | Meaning |
| --- | --- | --- |
| `dashboard_container` | `server-dashboard` | Dashboard container name. Protected from Docker actions |
| `gateway_container` | `server-dashboard-gateway` | Gateway container name |
| `docker_binary` | `/usr/bin/docker` | Docker CLI |
| `systemctl_binary` | `/usr/bin/systemctl` | |
| `systemd_run_binary` | `/usr/bin/systemd-run` | Used for one-shot reboots |
| `systemd_analyze_binary` | `/usr/bin/systemd-analyze` | Used to check schedule times |
| `timeout_binary` | `/usr/bin/timeout` | Bounds every command |
| `docker_unit` | `docker.service` | |
| `broker_unit` | `helix-privd.service` | |
| `power_state_root` | `/run/helix/power` | One-shot reboot records |
| `recurring_state_root` | `/var/lib/helix/power` | Recurring reboot schedule |
| `docker_cleanup_state_root` | `/var/lib/helix/docker-cleanup` | Docker cleanup schedule and last result |
| `systemd_unit_root` | `/etc/systemd/system` | Where schedule units are written |
| `broker_binary` | `/usr/local/libexec/helix-privd` | Used by schedule units |
| `broker_config_path` | `/etc/helix/privd.json` | Used by schedule units |
| `hook_services` | Plex `plexmediaserver.service`, Tailscale `tailscaled.service`, Pterodactyl `wings.service`, Jellyfin `jellyfin.service` | Hook cards and the systemd units they control, as `{ "id": "...", "unit": "..." }` |

#### `network`

| Key | Default | Meaning |
| --- | --- | --- |
| `docker_binary` | `/usr/bin/docker` | |
| `ufw_binary` | `/usr/sbin/ufw` | |
| `timeout_binary` | `/usr/bin/timeout` | |
| `proc_root` | `/proc` | Source of listening sockets |
| `state_root` | `/var/lib/helix/network` (installer: `/srv/helix/state/network`) | Records of Helix-owned firewall rules |
| `mutation_cooldown_seconds` | `2` | Minimum gap between firewall changes. 1 to 30 |

#### `packages`

| Key | Default |
| --- | --- |
| `dpkg_query_binary` | `/usr/bin/dpkg-query` |
| `apt_cache_binary` | `/usr/bin/apt-cache` |
| `apt_get_binary` | `/usr/bin/apt-get` |
| `apt_mark_binary` | `/usr/bin/apt-mark` |
| `timeout_binary` | `/usr/bin/timeout` |
| `apt_lists_root` | `/var/lib/apt/lists` |
| `apt_archives_root` | `/var/cache/apt/archives` |
| `reboot_required_path` | `/var/run/reboot-required` |
| `reboot_packages_path` | `/var/run/reboot-required.pkgs` |

#### `hook_installer`

| Key | Default |
| --- | --- |
| `os_release_path` | `/etc/os-release` |
| `curl_binary` | `/usr/bin/curl` |
| `gpg_binary` | `/usr/bin/gpg` |
| `apt_get_binary` | `/usr/bin/apt-get` |
| `dpkg_binary` | `/usr/bin/dpkg` |
| `systemctl_binary` | `/usr/bin/systemctl` |
| `docker_binary` | `/usr/bin/docker` |
| `timeout_binary` | `/usr/bin/timeout` |
| `apt_sources_root` | `/etc/apt/sources.list.d` |
| `apt_keyrings_root` | `/etc/apt/keyrings` |
| `share_keyrings_root` | `/usr/share/keyrings` |
| `staging_root` | `/run/helix/hook-installs` |

#### `helix_update`

| Key | Default | Meaning |
| --- | --- | --- |
| `github_owner` | `Riqqqque` | GitHub owner to check for releases |
| `github_repo` | `Helix` | GitHub repository |
| `compose_project_directory` | detected | Leave unset: the updater reads the Compose project and env file from the dashboard container's labels. If set, the folder must contain a `.env` file |
| `state_root` | `/var/lib/helix/updates` | Download, staging, rollback copies, and results |
| `curl_binary` | `/usr/bin/curl` | |
| `tar_binary` | `/usr/bin/tar` | |
| `timeout_binary` | `/usr/bin/timeout` | |
| `privd_install_path` | `/usr/local/libexec/helix-privd` | |
| `terminald_install_path` | `/usr/local/libexec/helix-terminald` | |
| `finalize_unit` | `helix-finalize-update.service` | Must be exactly this name |

#### `hub`

| Key | Default |
| --- | --- |
| `key_root` | `/var/lib/helix-hub` |
| `terminal_socket` | `/run/helix/terminal/terminal.sock` |
| `ssh_binary` | `/usr/bin/ssh` |
| `ssh_keygen_binary` | `/usr/bin/ssh-keygen` |
| `timeout_binary` | `/usr/bin/timeout` |

If the hub cannot start, the broker logs `machine hub disabled` and keeps
running without it.

### AMP credentials: `amp.json`

Only needed to show AMP-managed servers. Owned by root, mode `0600`, at most
16 KiB.

| Key | Default | Meaning |
| --- | --- | --- |
| `endpoint` | `127.0.0.1:8080` | AMP API address. Must be loopback |
| `public_panel_port` | the endpoint port | Port **Open AMP** links to |
| `username` | required | AMP login |
| `password` | required | AMP password |
| `instance_root` | `/home/amp/.ampdata/instances` | AMP instance folder, used to read ports |

Create the file before running the installer, or add `amp_credentials` to
`privd.json` yourself and restart the broker.

### Gateway: nginx

The gateway renders
[`deploy/nginx/default.conf.template`](https://github.com/Riqqqque/Helix/blob/main/deploy/nginx/default.conf.template)
from the `HELIX_*` gateway keys at start. Change the keys, not the template.

| Rule | Value | Result when it fails |
| --- | --- | --- |
| Host header | `HELIX_GATEWAY_HOST` or `HELIX_SECONDARY_GATEWAY_HOST` | `421 Misdirected Request` |
| Client address | `127.0.0.1`, `HELIX_ALLOWED_CLIENT_CIDR`, `HELIX_SECONDARY_ALLOWED_CLIENT_CIDR` | `403 Forbidden` |
| Origin on POST, PUT, DELETE | `HELIX_BROWSER_ORIGIN` or `HELIX_SECONDARY_BROWSER_ORIGIN` | `403 Forbidden` |
| Methods | GET, HEAD, POST, PUT, DELETE (terminal: GET only) | `403 Forbidden` |
| Request body | at most 6 MB (`client_max_body_size 6m`) | `413` |
| Client body and header timeouts | 10 seconds each | |
| Keep-alive | 30 seconds | |
| Upstream read and send | 15 seconds (terminal WebSocket: 8 hours read) | `504` |

`POST /api/v1/automation/server` with no Origin and no cookie is allowed through,
so server API token clients work from scripts. Forwarded-address headers from
clients are dropped. Responses carry `no-referrer`, `nosniff`, `DENY` framing,
and `noindex` headers.

## Related pages

- [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates)
- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery)
- [Troubleshooting and FAQ](https://github.com/Riqqqque/Helix/wiki/Troubleshooting-and-FAQ)
- [Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture)
