# Private-LAN Container Deployment

## Status

This is a private-alpha deployment example, not a supported installer. Use it
on a network you control, with current backups and a rollback plan. Do not bind
it to every interface, forward its port, place it behind a public tunnel, or
treat a successful container start as a completed security review.

No public domain is required. The source preview defaults to loopback; the
container layout exposes one exact private address through a constrained
gateway.

The checked LAN gateway serves plain HTTP. It is intended only for a trusted
private LAN and does not protect session traffic from a hostile peer on that
network. Any private-VPN or HTTPS terminator becomes part of the Host, Origin,
client-address, and cookie security boundary and needs separate review.

## Components

| Component | Runs as | Responsibility |
| --- | --- | --- |
| `dashboard` | Unprivileged container | `helixd`, compiled UI/API, local state, broker client |
| `gateway` | Unprivileged container sharing dashboard networking | Exact Host, Origin, client-CIDR, method, and request boundary |
| `helix-privd` | Separate root systemd service on the host | Closed typed host/storage/network/power/native/AMP operations |
| `helix-terminald@USER` | Separate unprivileged systemd service | Real PTY for one configured Linux user over a distinct group-protected socket |
| Native Minecraft | Separate Docker containers | Game processes and data; not children of the browser/dashboard |

Compose does not run the root broker. The dashboard receives only read-only
access to its group-protected Unix socket. The broker must be built, reviewed,
configured, and installed on the host separately.

## Security boundaries

- `helixd` listens on loopback inside the dashboard container namespace.
- The gateway is the only process on the published private socket.
- Both web containers drop Linux capabilities, use read-only roots and bounded
  logs/resources, and run without root.
- The gateway discards forwarded-client headers rather than trusting an
  undeclared proxy chain.
- The broker configuration names exact managed roots, native state/instance/
  backup roots, Docker binary, optional AMP credential file, and durable
  network-rule state.
- The broker service sandbox grants configured content roots explicitly. Hosts
  that enable selected APT updates, UFW mutations, or recurring reboot schedules
  must also retain the reviewed `/boot`, `/etc`, `/usr`, and `/var` exceptions
  from the service template. Package maintainer scripts and UFW need real host
  write/kernel authority; removing those exceptions makes those mutations fail.

The terminal socket uses both filesystem ownership/mode and Linux
`SO_PEERCRED`. The service rejects every client whose effective UID is not the
dashboard container's pinned UID `10001`, even if another process is somehow
placed in the terminal socket group. The privileged broker socket still relies
on its dedicated group boundary; independent kernel peer-credential validation
for that separate protocol remains a release gate.

Tmpfiles owns the shared `/run/helix` parent and its differently grouped broker
and terminal paths. The broker unit must not also claim that parent with
`RuntimeDirectory=`; systemd would recursively replace the terminal directory's
group whenever the broker starts. The terminal template uses
`WorkingDirectory=~`, and the daemon defaults to that NSS-resolved working
directory instead of assuming every account lives under `/home`.

## Prepare the target

Use a disposable or backed-up Linux target for the first deployment. Record
existing Docker workloads and do not stop, recreate, or prune unrelated
containers.

Before starting anything:

1. Build the exact checked source and record its revision.
2. Create separate dedicated `helix-broker` and `helix-terminal` groups and
   record both numeric GIDs. Never reuse the broker group for the terminal.
3. Choose absolute, private broker state, instance, backup, and managed-storage
   roots. Do not reuse a personal home directory or copy example roots blindly.
4. Create a root-owned broker configuration readable only by root.
5. Review the systemd unit's user, group, socket/state directories, executable,
   and `ReadWritePaths` against those exact choices.
6. Keep the deployment `.env` operator-owned and mode `0600`.
7. Keep world data, state, and backups out of the image and source tree.

The checked service expects the broker binary and configuration at the paths
declared in `deploy/helix-privd.service`. Its socket path must match the
read-only socket mount in `compose.yaml`. `HELIX_BROKER_GID` and
`HELIX_TERMINAL_GID` must match the two distinct host groups. The dashboard is
the only component added to both groups; the terminal user must never receive
access to the privileged broker socket. The terminal unit also pins
`--allowed-peer-uid 10001`; change that value only if the reviewed dashboard
image intentionally uses a different non-root UID, and keep it identical to
the inspected image user.

Build the broker from the same source as the dashboard:

```text
cargo build --locked --release -p helix-privd
cargo build --locked --release -p helix-terminal
```

Before enabling the installed unit, validate it with `systemd-analyze verify`
and inspect the effective sandbox. Add content roots one by one. Do not replace
the reviewed OS-mutation paths and configured roots with a writable `/` merely
to make one path work.

The terminal is optional. To enable it, install the exact same-source
`helix-terminald` binary, tmpfiles entry, and template unit, then instantiate the
unit only for the intended existing Linux login account:

```text
sudo install -m 0755 target/release/helix-terminald /usr/local/libexec/helix-terminald
sudo install -m 0644 deploy/helix-tmpfiles.conf /etc/tmpfiles.d/helix.conf
sudo install -m 0644 deploy/helix-terminald@.service /etc/systemd/system/helix-terminald@.service
sudo systemd-tmpfiles --create /etc/tmpfiles.d/helix.conf
sudo systemd-analyze verify /etc/systemd/system/helix-terminald@.service
sudo systemctl daemon-reload
sudo systemctl enable --now helix-terminald@<existing-linux-user>.service
```

Before those commands, create the dedicated system group named in tmpfiles and
record its numeric GID. Replace the user placeholder with one exact ordinary
local account; do not create a root instance and do not add that account to
`helix-broker`. Verify the resulting socket is a Unix socket owned by the
terminal user/`helix-terminal`, mode `0660`, beneath a `0770` terminal directory.
The checked unit pins dashboard peer UID `10001`; confirm the dashboard image
still uses `10001:10001` before starting it.

The terminal unit does not set `NoNewPrivileges` because it is meant to behave
like a normal login shell and preserve the account's existing `sudo` policy.
That is not permission for Helix to store or bypass the Linux password. If a
general shell is not wanted, leave the unit disabled and the Terminal page will
report unavailable without affecting other pages.

## Configure the private gateway

Copy `.env.example` to `.env`, then replace every example value. At minimum,
set:

- `HELIX_LAN_BIND_ADDRESS` to one exact private host address;
- `HELIX_LAN_PORT` to the chosen private port;
- `HELIX_GATEWAY_HOST` to the exact browser Host value;
- `HELIX_BROWSER_ORIGIN` to the exact private browser origin;
- `HELIX_ALLOWED_CLIENT_CIDR` to the narrow intended client subnet;
- `HELIX_BROKER_GID` to the dedicated broker-group GID; and
- `HELIX_TERMINAL_GID` to the separate terminal-socket group GID; and
- `HELIX_DATA_DIR` and `HELIX_BACKUP_DIR` to private persistent directories.

Leave all secondary-gateway values inert unless a separately constrained
private route is intentionally configured. Never use wildcard Host, Origin, or
client-CIDR values.

Validate the rendered deployment before building:

```text
docker compose config -q
docker compose build --pull
```

Review the rendered ports, bind address, mounts, group addition, image tags,
restart policies, and resource limits. The checked dashboard image pins
`10001:10001`; verify the built image still reports that exact user before
creating new empty bind roots:

```text
docker image inspect --format '{{.Config.User}}' helix-dashboard:<tag>
sudo install -d -m 0700 -o 10001 -g 10001 -- \
  <absolute-data-dir> <absolute-backup-dir>
```

Resolve and inspect both paths before running `install`. They must be the exact
new Helix roots, never `/`, a home directory, a storage-pool root, or a directory
that already contains unrelated data. Do not recursively `chown` an existing
installation as a shortcut and never use world-writable permissions.

Both bind roots must exist before `docker compose up`. The checked Compose file
sets `bind.create_host_path: false`, so a missing root fails clearly instead of
being auto-created as a root-owned directory. Stop, verify the exact path, and
create the intended empty root with mode `0700` and owner `10001:10001` rather
than loosening permissions.

Only then start the reviewed broker and Compose project:

```text
docker compose up -d --wait
docker compose ps
```

Do not use `--remove-orphans`, restart Docker globally, or prune shared images,
containers, or volumes as part of a Helix deployment.

## First owner

Create the one-time setup token only after the private state directory exists:

```text
docker compose stop gateway dashboard
docker compose run --rm --no-deps --entrypoint /app/bin/helixctl dashboard \
  --config /app/config/helix.toml setup-token
docker compose up -d --wait
```

The daemon stays stopped while the offline token is replaced. The token expires
after 15 minutes and is consumed by the first successful owner claim. Do not put
the token or password in `.env`, Compose, shell history, logs, screenshots, or
source.

After enrollment, verify login, protected reads, logout/login, broker status,
and that disabled or unavailable integrations say why rather than reporting
success.

## Verify the boundary

Confirm on the host that:

- the published dashboard socket uses only the configured private address;
- no public or wildcard socket was added;
- the broker socket has the intended owner, group, and restrictive mode;
- when enabled, the terminal socket uses its different group and the daemon
  runs as the chosen non-root Linux user with dashboard peer UID `10001`;
- the broker service and dashboard/gateway containers are active under their
  expected identities;
- unrelated Docker containers, UFW rules, services, and storage were unchanged;
- the Network page labels outside reachability `unverified`; and
- the package and Helix self-update controls remain unavailable.

The Start on boot setting changes only the exact configured dashboard/gateway
container restart policies and does not stop/start them. A later Compose
recreation can reapply the policy declared in `compose.yaml`.

## Tailscale

Helix can accept a second exact Host, Origin, and client CIDR for an already
configured private Tailscale route. It does not install, enable, authenticate,
or reconfigure Tailscale. Do not trust the whole carrier address range merely
because one expected node uses it, and do not treat Tailscale compatibility as
proof that remote access was configured or reviewed.

## Backup and rollback

Before replacing a working deployment, record image IDs and save the exact
Compose file, private environment, broker binary/config/unit, dashboard state,
native instance state, and backups into an operator-only rollback location.
Use `helixctl backup-state` for a verified critical-state snapshot when the
current database is healthy.

Keep previous images and the exact broker binary until the new revision, host
controls, server lifecycle, state integrity, and rollback path have been
verified. Restore only Helix-owned files and containers during rollback. Never
use a global Docker or filesystem cleanup as a rollback method.

## Remaining release gates

This layout still needs clean supported-host install/upgrade/uninstall matrices,
independent broker/gateway review, broker peer-identity hardening, full recovery
and fault drills, disposable live reboot/UFW testing, signed images/releases,
and a safe automated update path. Passing `docker compose up` does not close
those gates.
