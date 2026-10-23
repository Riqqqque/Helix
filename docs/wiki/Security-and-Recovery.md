# Security and Recovery

Helix can start containers, change firewall rules, install packages, and reboot
the machine. This page explains, in plain words, what protects those powers,
what Helix keeps safe for you, and how to back up and recover Helix itself. The
full technical model is in
[`docs/SECURITY.md`](https://github.com/Riqqqque/Helix/blob/main/docs/SECURITY.md)
and the recovery design in
[`docs/RECOVERY.md`](https://github.com/Riqqqque/Helix/blob/main/docs/RECOVERY.md).

> [!CAUTION]
> Helix 1.10 is a private-LAN release. It has not had an independent security
> review or a public-network review. Do not expose the dashboard to the
> internet, and do not let Helix hold the only copy of anything you care about.

## The threat model in plain words

Helix assumes:

- **Your LAN is trusted, but not every device on it is.** Anyone who can reach
  the dashboard port still needs your password. The dashboard uses plain HTTP,
  so someone who can watch your LAN traffic could read a session. That is why
  Helix stays on a network you control.
- **The browser is untrusted input.** Every request is checked on the server.
  Hiding a button in the UI is never what stops an action.
- **Game servers, plugins, mods, and downloads may be hostile.** They run in
  locked-down containers and never receive Helix's authority.
- **Root on the host is beyond Helix's protection.** Anyone with root can read
  memory, replace binaries, and edit any record. Helix cannot defend against
  that.

Current gaps, stated plainly: there is no multi-factor sign-in, no password
reset, no independently signed release key, no transactional rollback for APT
updates, and the broker socket is protected by file ownership and group
membership, not by a separate check of who is connecting.

## Private-LAN stance

- The dashboard is published on **one** private address you chose at install.
  The installer refuses public addresses.
- `helixd` itself listens only on `127.0.0.1` inside its container and refuses
  any other listen address. The gateway is the only thing on the LAN port.
- The gateway accepts only the exact Host you configured (others get `421`),
  only clients in your allowed range (others get `403`), and only your exact
  Origin for changes (others get `403`). It drops forwarded-address headers from
  clients instead of trusting them.
- To reach Helix from away from home, use a private network such as Tailscale.
  Never forward the dashboard port.

## Accounts, sessions, CSRF, and Origin

**First owner.** `helixctl setup-token` creates a random, single-use token that
expires after 15 minutes. Only its hash is stored. Creating the owner consumes
the token in the same step, so two people cannot both claim the server. Once an
owner exists, setup tokens are disabled for good.

**Passwords** are hashed with Argon2id. Changing the owner password signs out
every other session and invalidates every server API token.

**Sessions** are random tokens; only a hash is stored. The `helix_session`
cookie is `HttpOnly`, host-only, and `SameSite=Strict`. By default a session ends
after 30 minutes idle or 8 hours total. Settings can turn that expiry off, so a
session lasts until you log out or change your password, capped at 400 days.

**CSRF.** Every protected request needs the session cookie and the current
session-bound `X-Helix-CSRF` proof. The browser keeps only the proof, scoped to
this exact origin (scheme, address, and port), so a page on another port of the
same machine cannot read it.

**Origin.** Requests that change something must come from the configured Origin
and pass Fetch Metadata checks before Helix reads the body. Sign-in, setup, and
expensive storage analysis are rate limited, and failures are generic.

## Capabilities and roles

The server checks a named capability on every protected route.

| Capability | Allows |
| --- | --- |
| `system.view` | Host, network, and status views |
| `system.settings.write` | Helix and host settings |
| `system.power` | Reboots and start-after-boot |
| `system.packages.read`, `system.packages.write` | Linux updates |
| `network.firewall.read`, `network.firewall.write` | UFW and public access |
| `storage.files.read`, `storage.files.manage`, `storage.analyze` | Files and storage analysis |
| `games.view`, `games.manage`, `games.backups.manage` | Game servers and their backups |
| `dashboard.customize` | Home layouts and preferences |
| `users.manage` | Accounts |
| `terminal.open` | The browser terminal |
| `machines.view`, `machines.manage` | The machine hub |

Today there is one built-in role, **owner**, which holds every capability.
Fine-grained roles for other people are not available yet.

[Server API tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)
are separate. Each one is limited to the exact servers and operations you chose,
and none can reach host controls, users, or the terminal.

## The host broker

Everything that needs root goes through `helix-privd`, a separate systemd
service on the host.

- It has no network listener. It listens only on `/run/helix/privd.sock`, which
  only root and the `helix-broker` group can open. The dashboard container is
  added to that group; the terminal user must never be.
- Requests are a closed list of typed operations with a bounded size, such as
  "start server with this ID" or "delete this exact Helix firewall rule". There
  is no request that runs a command string, picks a program, or names an
  arbitrary systemd unit.
- It checks everything again itself: configured folders, exact IDs and container
  names, value ranges, current state, and whether another operation is running.
- Docker, UFW, APT, and systemd are run with fixed argument lists, never through
  a shell, and with time limits.
- Its configuration must be owned by root and private, or it refuses to start.
- It runs in a systemd sandbox with an explicit list of writable paths. Package
  updates, UFW, and schedules need write access to `/etc`, `/usr`, `/var`, and
  `/boot`, so those features carry real root power even though the requests are
  typed.

The optional browser terminal is separate from the broker. It runs as one
ordinary Linux user, asks for your Helix password for each session, and accepts
connections only from the dashboard's exact user ID. Helix records that a
session opened or closed, never what you typed or saw. See
[Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal).

## Token vault

Passwords and session tokens are stored only as hashes. Server API tokens are
the one exception, so the owner can view a token again:

- a one-way fingerprint is used to check each token;
- an encrypted copy (XChaCha20-Poly1305, envelope-encrypted under a master key)
  is kept for **View token**;
- the master key is `secrets/token-vault.key` in the dashboard data folder
  (`/srv/helix/dashboard/data`), mode `0600`, outside the `state/` database
  folder;
- viewing a token needs your current password and is written to the audit log;
- revoking or rotating a token deletes its stored copy.

A copied state database or a `helixctl backup-state` snapshot does not reveal
any token. Someone with the whole data folder could.

## Game server containers

Each Helix game server runs in its own Docker container:

- under its own numeric user ID, different for every server;
- with every Linux capability dropped and `no-new-privileges` set;
- with a memory limit and no extra swap, and a process limit;
- with only that server's folder mounted;
- with bounded container logs; and
- for Minecraft, with a read-only root filesystem and RCON bound to
  `127.0.0.1`.

Game servers keep running when the dashboard, gateway, or broker restarts, and
when the browser is closed. Console output is captured by the broker, not the
browser.

## File access guarantees

- **Configured folders only.** Files and Storage work only inside the
  `managed_roots` in the broker configuration. A server's Files tab cannot leave
  that server's folder.
- **No symlinks.** The broker opens every path step by step and refuses to
  follow links, so a link that a game or plugin places in its folder cannot
  reach host files.
- **Revisions.** Text edits carry the revision you read. If the file changed
  since, the save is refused instead of overwriting someone else's change.
- **Recoverable deletion.** Deleting a file moves it to a `.helix-trash` folder
  (in the server's folder for a server's Files tab, or at the top of the
  managed folder for Storage) instead of erasing it. Deleted
  backups go to the backup trash and removed servers to **Removed and hidden**.
- **Bounded work.** Listings, reads, uploads, and storage analysis have size and
  time limits. Storage analysis never follows links and stays on one
  filesystem.
- **Safe copies.** Copying a setup between servers reads the source inside a
  locked-down container with no network, running as the source server's user.

Only the backup trash empties itself: deleted backups are removed for good 7
days after deletion by default (per-server setting, `0` keeps them). The Files
trash and removed servers stay until you empty them.

### Staged changes stay out of the game's reach

Files you upload through the server API while a server runs are staged instead
of written into the live folder. They are kept in Helix's own state folder
(`/srv/helix/state/instances/pending/`), which the game container cannot see,
and nothing in the server folder changes until they are installed on the next
start, restart, or **Deploy**. If the pending store is not private to Helix,
Helix refuses to use it. See
[Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).

### Risky changes take a backup first

Server software updates, modpack updates, restores, copies between servers, and
deploys take a full backup first and restore it automatically if the server does
not come back.

## Audit logs

Helix records sign-in and session events, terminal session events, server API
token use (which token, which server, the permission decision, and the result),
and token views. It never records passwords, tokens, keys, file contents,
console commands, or terminal input. Older events are pruned in bounded steps.

The records live in the dashboard state database and are included in state
snapshots. There is no audit viewer in the dashboard yet. The log is local and
can be edited by root, so it is not tamper evidence.

## What to back up

Helix's own backups live on the same disk as the servers. Copy these off the
machine on a schedule:

| Path | Contents |
| --- | --- |
| `/srv/helix/instances/` | Game server folders and worlds |
| `/srv/helix/backups/` | Game server backups |
| `/srv/helix/state/` | Server definitions, console history, staged changes, firewall rule records |
| `/srv/helix/dashboard/data/` | Accounts, preferences, tokens, audit, and `secrets/token-vault.key` |
| `/srv/helix/dashboard/backups/` | State snapshots |
| `/etc/helix/` | Broker configuration and AMP credentials |
| `/opt/helix/helix.env` | Address, port, and gateway settings |

These contain credentials. Store the copy somewhere private and encrypted.

## Backing up Helix's own state

`helixctl backup-state` writes a consistent, verified copy of the dashboard state
database while Helix keeps running. Run it inside the dashboard container:

```bash
sudo docker exec server-dashboard /app/bin/helixctl --config /app/config/helix.toml \
  backup-state /var/lib/helix-backups/helix-state-2026-10-22.db
```

The file appears on the host in `/srv/helix/dashboard/backups/`. The
destination must not already exist. The in-app updater takes the same kind of
snapshot (`helix-update-<time>.db`) before every update.

A snapshot contains accounts, sessions, preferences, server API token records,
and audit events. It does **not** contain `secrets/token-vault.key`, game
servers, worlds, or backups. Keep the key with the snapshot if you want
**View token** to work after a restore.

To check the live database:

```bash
sudo docker exec server-dashboard /app/bin/helixctl --config /app/config/helix.toml doctor
```

Add `--full` for SQLite's full integrity check.

## Restoring Helix's own state

There is no restore command yet, and a full restore has not been drilled. A
snapshot is an ordinary SQLite database, so restoring it is a manual swap:

1. Stop the dashboard:
   `sudo docker stop server-dashboard-gateway server-dashboard`.
2. Move the current `/srv/helix/dashboard/data/state/` folder aside. Keep it.
3. Create a new `state/` folder (owner `10001:10001`, mode `0700`) and copy the
   snapshot into it as `helix-state.db` (owner `10001:10001`, mode `0600`).
4. Start the dashboard: `sudo docker start server-dashboard server-dashboard-gateway`.
5. Sign in and check your servers and settings.

Restoring an older database brings back the passwords and tokens that were valid
then. After a security incident, change the owner password and rotate tokens
again. A snapshot from a newer Helix version will not open in an older one.

Game servers do not depend on this database. Their definitions live in
`/srv/helix/state/instances/`, so they stay in place through a dashboard state
restore.

## Disaster recovery

If the machine is lost, recovery means a new install plus your off-host copies.
Fresh-machine recovery is not automated and has not been drilled end to end.

The dependable path for game servers is to start fresh and bring the worlds
back in:

1. Install Helix on the new machine and create the owner.
2. Copy your saved server folders (or unpacked backups) into a folder Helix
   manages, for example one you passed to the installer with `--storage`.
3. Use **New server → Copy an existing server → Folder on this host** to create a
   Helix server from each one. See
   [Server Migration](https://github.com/Riqqqque/Helix/wiki/Server-Migration).

To bring back the dashboard itself (accounts, layouts, tokens), restore a state
snapshot as described above. Old server API tokens point at old server IDs, so
rotate them afterwards.

Copying all of `/srv/helix` back onto a new install restores server definitions
too, but Docker containers are not part of that copy. Servers whose container is
gone show "Execution container is missing", and there is no tested way yet to
recreate them in place. Prefer the copy path above for anything important.

Keep at least one off-host copy that Helix cannot delete. Helix's local backups
protect against mistakes and failed updates, not a dead disk.

## Reporting a vulnerability

Report security problems privately through
[GitHub private vulnerability reporting](https://github.com/Riqqqque/Helix/security/advisories/new),
never in a public issue. The
[security policy](https://github.com/Riqqqque/Helix/blob/main/SECURITY.md) lists
what a useful report includes. Remove passwords, tokens, cookies, private
addresses, hostnames, storage paths, and world data from anything you send. No
response time is promised for this private-LAN line.
