# Automation API

The automation API lets a script manage chosen game servers with a
[server API token](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)
instead of your dashboard login. This page lists every operation a token can
run, its fields and permission, and walks through complete examples.

For login-based access, error shapes in general, and the Python client, see
[API and Integrations](https://github.com/Riqqqque/Helix/wiki/API-and-Integrations).

> [!CAUTION]
> Helix is a private-LAN release. Run automation from machines on your own
> network or through an SSH tunnel. Do not expose the dashboard to the
> internet to reach it from elsewhere.

## Endpoints

| Method | Path | Use |
| --- | --- | --- |
| `POST` | `/api/v1/automation/server` | Run one operation. The JSON body names it |
| `GET` | `/api/v1/automation/jobs` | List the last 256 jobs this token started |
| `GET` | `/api/v1/automation/jobs/{job_id}/wait?timeout_seconds=N` | Wait up to 12 seconds for one of this token's jobs to finish |

Every request sends:

```http
Authorization: Bearer <token>
Content-Type: application/json
```

Do not send `Cookie` or `Origin`. A request with either header, or with more
than one `Authorization` header, is refused. Tokens cannot call any other
route.

## The operation envelope

The body of `POST /api/v1/automation/server` is one JSON object. `operation`
picks the operation, and the other fields belong to it:

```json
{"operation": "server_action", "instance_id": "helix:EXACT-UUID", "action": "restart"}
```

- `instance_id` is the server's exact ID, including its prefix (`helix:` for
  native servers). It must be on the token. The server page's **Advanced** tab
  shows it.
- File commands and backup downloads put their command in a nested `request`
  object.
- Unknown operations, unknown fields and wrong types are refused with
  `400 invalid_json`.
- The body limit is 5 MiB.

The response is the operation's result, as plain JSON with no wrapper. Long
operations return a job instead:

```json
{"job_id": "0b9a6c7e-5d0f-4f7e-8f0e-2f6d2b1c9a41", "reused": false}
```

### What Helix checks, in order

1. A free automation slot (two requests at a time across all tokens).
2. The bearer token: valid, not expired, not revoked, and issued under your
   current password and account.
3. The token's rate limit: 600 requests per minute.
4. The body parses as a known operation.
5. The server is on the token and the token has the operation's permission. For
   a transfer, both servers must be on the token.
6. The decision is written to the audit log.
7. Helix runs the operation and writes the result to the audit log. If the
   result has a `job_id`, the job is linked to this token.

Once step 5 passes, Helix finishes the operation and its bookkeeping even if
your connection drops.

## Operations

`instance_id` is required for every operation except `job_status`.

### Server information and logs

| Operation | Permission | Fields | Returns |
| --- | --- | --- | --- |
| `server_capabilities` | `view` | none | What this server supports (see below) |
| `server_detail` | `view` | none | Native server detail: status, software, version, ports, memory, `tps` when known, modpack provenance |
| `server_logs` | `logs` | `lines` (integer, 1–1000; larger values are clamped) | `{instance_id, lines[], collected_at_unix_ms}` |
| `server_log_history` | `logs` | `lines` (1–500), `cursor` (string or null) | A page of retained console history: `entries[]`, `next_cursor`, `has_more`, retention info. Pages run from newest to older |

For a native server, `server_capabilities` returns `manager: "helix"`, `game`,
`actions` (all lifecycle actions; `update` is missing for custom Minecraft
JARs), `file_actions`, `max_upload_bytes`, `max_chunk_bytes`,
`console_commands`, `settings`, `logs`, `backups`, `backup_download`,
`valheim_management`, and `runtime_changes`. For an imported AMP server it
describes what the adapter supports. Read it before choosing operations.

> [!NOTE]
> Since 1.10.2 the capabilities response says
> `"file_mutations_require_stopped": false` and
> `"file_changes_staged_while_running": true`: file changes on a running server
> are staged (see [Staged changes](#staged-changes-on-a-running-server)). Older
> brokers still report `true`.

### Lifecycle

| Operation | Permission | Fields | Returns |
| --- | --- | --- | --- |
| `server_action` | `start`, `stop`, `restart`, `kill`, `update` or `backups.write`, matching `action` | `action`: `start`, `stop`, `restart`, `kill`, `update`, or `backup` | A job for native servers |

What each action does on a native server:

| Action | What happens | `result.detail` |
| --- | --- | --- |
| `start` | Installs staged file changes, starts the server, waits until the game is ready | `online`, `already_running`, `applied_pending`, `pending_error` |
| `stop` | Java Minecraft first runs `save-all flush`; if the save cannot be confirmed the server stays running. Then a clean stop | `online: false`, `already_stopped` |
| `restart` | Stop, install staged changes, start, wait until ready | `online`, `previously_running`, `applied_pending`, `pending_error` |
| `kill` | Force-stops the container. Can lose unsaved progress | `online: false`, `already_stopped` |
| `update` | Updates the server software with its own checks and backup. A modpack server moves to the newest compatible release with a full safety backup and rollback | Update detail |
| `backup` | Takes a backup. See [Backup modes](#backup-modes) | `backup_id`, `mode`, `retention_trashed`, `retention_error` |

The job `result` is `{instance_id, action, accepted, detail}`.

If a staged change cannot be installed, `start` and `restart` still start the
server on its current files and report the reason in `pending_error`. Use
`deploy_pending_changes` when you want a failed install to stop and roll back.

`kill` runs beside a stop or restart that hangs, but is refused during a
backup, update, restore or marketplace install. Imported AMP servers do not
support `kill`.

#### Backup modes

| `mode` | When |
| --- | --- |
| `live` | A running Java Minecraft server with a working console. Helix turns off autosave, flushes the world, archives the folder while players stay connected, and always turns autosave back on |
| `stopped` | Any other running server, or a Minecraft server whose console did not answer. Helix stops it, backs up, and starts it again |
| `offline` | The server was already stopped |

### Console and settings

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `server_console` | `console` | `command` (one line, under 512 bytes; a leading `/` is dropped) | Minecraft (RCON) and Hytale only. The server must be running. Returns the command's response |
| `server_settings` | `settings` | none | Settings, `expected_revision`, field metadata and restart state. Minecraft only |
| `update_server_settings` | `settings` | `settings`: `expected_revision`, `motd`, `game_mode` (`survival`, `creative`, `adventure`, `spectator`), `difficulty` (`peaceful`, `easy`, `normal`, `hard`), `max_players`, `view_distance`, `simulation_distance`, `player_idle_timeout`, `online_mode`, `pvp`, `allow_flight`, `white_list`, `enforce_white_list`, `spawn_protection`, `game_port`, `memory_mb`. All fields are required | Revision-guarded. A stale revision is refused; read again and reconcile |
| `valheim_manage` | `settings` for `status` and `save_settings`; `update` for the rest | `request` with `action`: `status`; `save_settings` (`expected_revision`, `settings`); `package` (`reference`); `install` (`reference`); `set_mod_enabled` (`package`, `enabled`); `remove_mod` (`package`); `check_updates`; `update_game` (`repair`) | Valheim only. Install, mod changes, update checks and game updates are jobs. See [Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim) |

The console records the command line in the server's console history. The
audit log never stores the command text.

### Resources and ports

Tokens can manage every port a server uses: the game port through
`update_server_settings` (`game_port`), extra ports for voice chat, maps, and
Bedrock players through `set_native_extra_ports`, and whether the game port is
open to the internet through `set_server_network_exposure`.

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `set_native_memory` | `settings` | `memory_mb` (integer) | Same bounds as creation, for example Minecraft 1024–65536. Recreates the container; starts it again only if it was running |
| `set_native_cpu` | `settings` | `cpu_millis` (0 for no cap, or 250–128000) | Docker `--cpus` in thousandths of a core. Recreates the container |
| `set_native_extra_ports` | `settings` | `ports`: up to 16 of `{port, protocol, label}`. `port` is 1024 or higher and listed once; `protocol` is `tcp`, `udp` or `both`; `label` is optional, up to 40 characters | Extra host ports published straight to the container, for voice chat, maps and web panels |
| `set_native_start_on_boot` | `settings` | `enabled` (boolean) | Sets the container restart policy. Does not start or stop the server now |
| `set_native_browser_listing` | `settings` | `list_on_browser` (boolean) | V Rising server-list visibility. Reports `restart_required` when running |
| `set_server_address` | `settings` | `address` (string up to 255 characters, or null/empty to clear) | Saves your own address for players, such as `play.example.com:25565`. Letters, digits, `.`, `-`, `_`, `:`, `[` and `]` only; no protocol. Shown with a copy button and returned as `custom_address` in `server_detail` |
| `set_server_network_exposure` | `network` | `enabled` (boolean) | Adds or removes Helix-owned host firewall rules for the game port. Never changes your router |
| `change_native_runtime` | `update` | `version` (string, or null to repair the current version), `expected_version`, `expected_build`, `confirmation_name` (the exact server name) | Published Minecraft software only. A job |

### Marketplace

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `server_marketplace_search` | `view` | `query` (string, up to 120 bytes), `offset` (0–10000), `limit` (1–50), optional `provider` (`modrinth` default, or `curseforge`), optional `catalog` (`content` default, or `modpacks`) | Filtered to the server's game version and loader |
| `server_marketplace_project` | `view` | `project_id`, optional `provider` | Project and version compatibility detail |
| `install_server_marketplace_content` | `files.write` | `project_id`, `version_id` (string, or null for the newest compatible release), optional `provider`, optional `restart_server` | A job. Writes checksum-verified JARs into `plugins/` or `mods/` with required dependencies. Result includes `restart_required` |

> [!NOTE]
> Without `restart_server`, marketplace installs write the files into the server
> folder and leave a running server running; restart it yourself when the result
> says `restart_required: true`. With `"restart_server": true` (1.10.2 and later)
> a running server is stopped before the files change, started again, and
> checked; if it does not come back, the previous files are put back and it
> starts again. The result then says `runtime_validation_performed: true`.

### Backups

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `list_backups` | `backups.read` | none | Backups, trash, keep rules and trash rules (see below) |
| `server_backup_download` | `backups.read` | `backup_id`, `request`: `{"action":"stat"}` or `{"action":"download","offset":N,"length":N,"expected_revision":"..."}` | Exports the exact `.tar.gz` archive in chunks of up to 1 MiB |
| `server_action` with `backup` | `backups.write` | `action: "backup"` | A job. See [Backup modes](#backup-modes) |
| `restore_backup` | `backups.write` | `backup_id` | A job. Stops the server, takes a safety backup, restores, and starts it again if it was running |
| `trash_backup` | `backups.write` | `backup_id` | Moves a backup to this server's backup trash |
| `restore_trashed_backup` | `backups.write` | `trash_id` | Undoes a deletion |
| `purge_backup_trash` | `backups.write` | `trash_id` | Deletes one trashed backup forever |
| `purge_backup_trash_older_than` | `backups.write` | `days` (0–3650) | Deletes every trashed backup deleted more than `days` ago. `0` empties the trash. Returns `purged`, `purged_bytes`, `older_than_days` |
| `set_backup_policy` | `backups.write` | `keep_count` (0–50), `keep_days` (0–365), optional `trash_days` (0–365) | `0` means no limit for keep rules. Applies the rules at once and returns the catalog with `retention_trashed` and `trash_emptied` |
| `prune_backups` | `backups.write` | none | Applies the saved keep rules now |

`list_backups` returns:

| Field | Contents |
| --- | --- |
| `backups[]` | `id`, `created_at_unix_ms`, `size_bytes`, `definition_present`, newest first. Up to 2,048, with `backups_truncated` |
| `trash[]` | `trash_id`, `backup_id`, `trashed_at_unix_ms`, `created_at_unix_ms`, `undo_available`, `purge_eligible_at_unix_ms`, `automatic_purge_enabled`, `size_bytes` |
| `trash_policy` | `purge_after_days`, `server_setting` (null when the server follows the host default), `host_default_days`, `automatic_purge_enabled`, `note` |
| `policy` | `keep_count`, `keep_days`, `keep_count_max` (50), `keep_days_max` (365), `note` |

Keep rules move extra backups to the trash; they never delete them. The trash
then empties itself: trashed backups are deleted for good after `trash_days`
(7 by default, set per host). `0` keeps them until you delete them. Helix checks
every hour and after each backup. Omit `trash_days` in `set_backup_policy` to
leave the trash rule unchanged. See [Backups](https://github.com/Riqqqque/Helix/wiki/Backups).

#### Downloading a backup

1. Send `{"action":"stat"}`. The reply has `size` and `revision`.
2. Send `download` with `offset: 0`, `length` up to `1048576`, and that
   `revision` as `expected_revision`.
3. Each reply has `data_base64`, `sha256` of this chunk, `offset`,
   `next_offset`, `size`, `revision` and `eof`. Check the chunk hash, append
   the bytes, and continue from `next_offset` until `eof` is true.

The revision is checked before and after every chunk, so a backup that changes
under you fails instead of producing a mixed file.

### Files

| Operation | Permission | Fields |
| --- | --- | --- |
| `server_files` | `files.read` for `list`, `stat`, `read`, `download`, `pending_list`; `files.write` for every other action | `request` with an `action` (below) |

See [Server files](#server-files) for every action.

### Deploying staged changes

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `deploy_pending_changes` | `restart` | optional `options`: `backup` (default true), `start` (default true), `expect_log` (string, 1–200 characters), `revert_on_failure` (default true) | A job. Native servers only. See [Deploy](#deploy) |

### Copying between servers

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `server_transfer_preflight` | `files.read` and `view`; both servers on the token | `instance_id` (source), `target_id`, `parts` | Changes nothing. Reports files, sizes, errors and warnings |
| `transfer_server_content` | `files.read` and `files.write`; both servers on the token | `instance_id` (source), `spec`: `target_id`, `parts`, `confirmation_name` (the target's exact name), optional `remove_missing_jars` (default false) | A job |

`parts` is a list of up to 8 unique values:

| Part | Copies |
| --- | --- |
| `plugins` | Plugin JARs and config files, without databases, logs, caches and player folders |
| `plugin_data` | Everything under `plugins/`, including databases and player files |
| `mods` | `mods/*.jar` |
| `configs` | `bukkit.yml`, `spigot.yml`, Paper, Purpur, Pufferfish and Leaves files, `config/`, `defaultconfigs/` |
| `server_properties` | Gameplay settings. The target keeps its ports, RCON, `server-ip`, `level-name`, seed, player limit, MOTD and whitelist settings |
| `datapacks` | The world's `datapacks/` folder |
| `player_lists` | `whitelist.json`, `ops.json` and ban lists, replacing the target's |
| `worlds` | The world, nether and end folders, replacing the target's worlds |

The target must be a Helix-managed server. The copy backs up the target, stops
it, writes the files, and starts it again if it was running. If anything fails,
including the target not starting, Helix restores the backup. A plugin or mod
JAR replaces the target JAR that declares the same name; set
`remove_missing_jars` to also delete target JARs the source does not have. See
[Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers).

### Removal

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `trash_native_server` | `remove` | `confirmation_name` (the exact server name) | Stops and removes the workload and moves its data to **Removed servers**, where the owner can restore it |

### Job status

| Operation | Permission | Fields | Notes |
| --- | --- | --- | --- |
| `job_status` | none beyond owning the job | `job_id` | Only jobs this token started, on a server still on the token |

### Not available to tokens

Tokens cannot create servers, list all servers, restore or purge removed
servers, change port pools, run migrations, use storage, network, package,
Docker, hook, terminal, machine or Strand routes, or manage tokens. These
requests are refused with `403 authorization_denied` and logged as
`api_token.unsupported`.

Operations other than `server_action` and `server_capabilities` work only on
Helix-managed servers. On
an imported AMP server they fail with *deep management is available only for
Helix-owned servers*.

## Server files

Every path is relative to the server's folder and uses `/`. An empty path means
the server folder itself. Helix refuses `..`, absolute paths, symbolic links,
hard-linked files, sockets and devices, and names starting with
`.helix-api-`. The server folder and its `.helix-trash` folder cannot be
renamed or removed. Access goes through directory handles, so swapping a folder
for a link cannot redirect a write elsewhere on the host.

### Actions

| Action | Fields besides `action` | Result |
| --- | --- | --- |
| `list` | `path`, `limit` (1–200), optional `cursor` | `entries[]` (`name`, `path`, `size`, `kind`: `file`, `directory`, `symlink` or `other`, `restricted`), `next_cursor`, `omitted_entries` |
| `stat` | `path` | `path`, `kind`, `size`, `revision` |
| `read` | `path` | UTF-8 `content` and `revision`. Up to 1 MiB |
| `download` | `path`, `offset`, `length` (1 byte to 1 MiB), `expected_revision` | `data_base64`, `sha256`, `offset`, `next_offset`, `size`, `revision`, `eof` |
| `create` | `path` | A new empty file. Never overwrites |
| `mkdir` | `path` | One new folder; the parent must exist |
| `write` | `path`, `content` (up to 1 MiB), `expected_revision` | Replaces a text file atomically; returns `revision` and `recovery_path` |
| `move` | `path`, `destination`, `expected_revision` | Renames or moves inside this server. Never overwrites. Needs a stopped server |
| `trash` | `path`, `expected_revision` | Moves to `.helix-trash`; returns `recovery_path` |
| `upload_file` | `path`, `data_base64` (up to 3 MiB decoded), `sha256`, optional `expected_revision` | One-request upload |
| `upload_begin` | `path`, `size` (0 to 8 GiB), `sha256`, optional `expected_revision` | `upload_id`, `bytes_written`, `max_chunk_bytes`, `idle_timeout_seconds`, `resumed`, `completed` |
| `upload_chunk` | `upload_id`, `offset`, `data_base64` (1 byte to 1 MiB decoded) | `bytes_written`, `size`, `replayed` |
| `upload_status` | `upload_id` | `path`, `bytes_written`, `size`, `sha256` |
| `upload_finish` | `upload_id` | Publishes the file, or stages it on a running server |
| `upload_abort` | `upload_id` | `aborted: true` |
| `pending_list` | none | `changes[]`, `count`, `total_bytes` |
| `pending_discard` | optional `path` | `discarded[]`, `count` |

### Revisions

A revision is an opaque string from `stat`, `read`, `write` or an upload. Do
not build or parse one. Pass it back as `expected_revision` to prove you are
changing the version you saw. If the file changed since, Helix refuses with
*file revision conflict; reread the file and reconcile before changing or
downloading it*.

- Omit `expected_revision` (or send null) on `upload_file` and `upload_begin`
  to create a new file. If the path exists, Helix refuses with *destination
  exists; read its revision and explicitly request replacement*.
- Pass the current revision to replace an existing file on purpose.
- On a running server, a staged change has its own revision, `pending:…`. Pass
  it to change that staged file again.

### Recovery paths

Every replacement and every `trash` keeps the old file. The result's
`recovery_path` points to it, for example `.helix-trash/MyPlugin.jar.…`. To
undo:

1. `stat` the recovery path to get its revision.
2. If the original path is taken, `trash` what is there first. `move` never
   overwrites.
3. `move` the recovery path back to the original path with that revision.

File trash is never emptied automatically and uses disk space. Full server
backups are separate from per-file recovery.

### Uploads

**`upload_file`** sends a whole file of up to 3 MiB in one request. Helix checks
the SHA-256 and publishes the file atomically. If the destination already holds
exactly that content, the reply has `completed: true` and `unchanged: true` and
nothing is written. That makes a repeat after a lost response safe on a stopped
server.

**Chunked uploads** handle anything from 0 bytes to 8 GiB:

1. `upload_begin` with `path`, total `size`, the whole file's `sha256`, and
   `expected_revision` when replacing. The reply gives an `upload_id`,
   `bytes_written` (where to continue from) and `max_chunk_bytes` (1 MiB).
2. `upload_chunk` with `offset` equal to `bytes_written` and up to
   `max_chunk_bytes` of data. Each reply gives the new `bytes_written`.
3. `upload_finish` when every byte is sent. Helix checks the size and SHA-256
   before anything becomes visible.

If `upload_begin` returns `completed: true`, the destination already matches
and there is nothing to send.

Resume rules:

- Repeating `upload_begin` with the same path, size, SHA-256 and expected
  revision returns the same `upload_id` with `resumed: true` and the
  acknowledged `bytes_written`. Continue from there.
- Repeating a chunk Helix already has is fine if the bytes match; the reply
  says `replayed: true`. Different bytes are refused.
- A chunk at the wrong offset is refused with *chunk offset or size does not
  match this upload; read upload_status before resuming*. Call
  `upload_status`.
- An `upload_begin` for the same path with different details discards the old,
  unfinished upload. The destination is not touched.
- An upload with no request for 10 minutes expires. A broker restart also drops
  it. Then `upload_chunk` says *upload is not active for this server*; start
  again with `upload_begin`.
- Only two chunked uploads can be active at a time across Helix. A third is
  refused with *two uploads are already active; finish or abort one first*.
- `upload_abort` discards the unfinished bytes.

Unfinished uploads use unnamed temporary files, so a crash never leaves a half
plugin or world file behind. The server's filesystem must support Linux
`O_TMPFILE`, hard links and atomic exchange, such as ext4 or XFS. Docker
writable layers and some network filesystems do not.

### Staged changes on a running server

On a stopped server, `create`, `write`, `trash`, `upload_file` and
`upload_finish` change the live files at once. On a running server they are
**staged** instead. Nothing the game can see changes until the next start,
restart or deploy, which installs every staged change while the server is
stopped. Staged files are kept in Helix's own state folder, out of the game's
reach.

A staged change replies like this:

```json
{
  "path": "plugins/MyPlugin.jar",
  "pending": true,
  "kind": "write",
  "size": 482113,
  "sha256": "9f2c…",
  "revision": "pending:4c1d…",
  "completed": true,
  "unchanged": false,
  "applies": "on the next start, restart, or deploy"
}
```

Rules for staging:

- `write` needs `expected_revision`: the live file's revision, or the
  `pending:…` revision of a change already staged for that path.
- A new file (no `expected_revision`) is refused if the live path exists, or if
  a change is already staged for it: *a change is already staged for PATH; pass
  its revision pending:… to replace it*.
- If the live file already has exactly the uploaded content, nothing is staged
  and the reply has `unchanged: true`. Any change staged for that path is
  dropped.
- `trash` stages a delete (`kind: "delete"`, revision `pending:delete`). If the
  path only existed as a staged new file, the staged file is dropped instead
  and the reply has `discarded_staged_file: true`.
- `mkdir` runs at once even while the server runs. Folders that a staged file
  needs are created when it is installed.
- `move` is refused while the server runs: *moving files needs the server
  stopped*. Upload the file under its new path and trash the old one instead;
  both are staged.
- A chunked upload begun on a running server is staged when it finishes. One
  begun while stopped and finished after a start is staged too.
- A direct change on a stopped server replaces anything staged for that path.
- Up to 500 changes can wait per server.

`pending_list` shows what is waiting:

```json
{
  "changes": [
    {
      "path": "plugins/MyPlugin.jar",
      "kind": "write",
      "size": 482113,
      "sha256": "9f2c…",
      "revision": "pending:4c1d…",
      "replaces_existing": true,
      "staged_at_unix_ms": 1761100000000
    }
  ],
  "count": 1,
  "total_bytes": 482113
}
```

`pending_discard` with a `path` drops that change; without one it drops them
all. Naming a path with nothing staged is refused.

When changes are installed, replaced and deleted files go to `.helix-trash` as
usual. Each installed change is reported with `path`, `kind`, `recovery_path`,
any `created_folders`, and `changed_since_staged`, which is true when the live
file changed after you staged (for example, a plugin rewrote its config on
shutdown). The staged version still wins and the other copy is in the trash.
If any change fails to install, Helix puts back the ones it already installed
and leaves every change staged.

> [!NOTE]
> On a running server, repeating an identical `upload_file` or `upload_begin`
> (same path, size, and SHA-256) after the first one succeeded returns the
> staged change with `"unchanged": true` (1.10.2 and later), so a lost response
> is safe to retry. Different content at the same path still needs the staged
> change's `pending:` revision.

### Locks

Reads take no lock. File changes can run side by side, but wait for a backup,
restore, update, deploy or lifecycle action on the same server. A file change
that waits more than 8 seconds fails with a message naming the operation that
holds the server.

## Deploy

`deploy_pending_changes` installs the server's staged changes as one job. Use
it after uploading plugins or configs to a running server.

```json
{
  "operation": "deploy_pending_changes",
  "instance_id": "helix:EXACT-UUID",
  "options": {"expect_log": "Enabling MyPlugin"}
}
```

| Option | Default | Effect |
| --- | --- | --- |
| `backup` | `true` | Back up first. A running Java Minecraft server is backed up live; other servers are stopped for it |
| `start` | `true` | Start the server afterwards. `false` matters only for a server that was already stopped; a running server is always started again |
| `expect_log` | none | Console text, 1 to 200 characters. After the game is ready, Helix waits up to a minute for a console line that contains it |
| `revert_on_failure` | `true` | If the server does not come back, put the previous files back and start it again |

The job runs these stages:

1. Check that something is staged. If not, it fails with *nothing is staged for
   this server; upload or write files first*.
2. Back up. If the backup fails, nothing is deployed.
3. Stop the server.
4. Install the staged changes. If one fails, the others are put back, the
   changes stay staged, and a server that was running is started again.
5. Start the server and wait until the game is ready. Minecraft must answer a
   status ping; other games write their ready marker.
6. If `expect_log` is set, wait up to 60 seconds for it.

If step 5 or 6 fails and `revert_on_failure` is on, Helix stops the server, puts
back the previous files, starts it again if it was running, and fails the job
with a message that says what happened and names the backup. The deployed
versions stay in the Files trash. With `revert_on_failure` off, the job fails
and leaves the new files in place.

A successful job's `result`:

| Field | Meaning |
| --- | --- |
| `instance_id` | The server |
| `deployed` | `true` |
| `applied[]` | Each installed change: `path`, `kind`, `recovery_path`, `created_folders`, `changed_since_staged` |
| `backup` | `{backup_id, mode}` (`live`, `stopped` or `offline`), or null when `backup` was false |
| `online` | Whether the server was started |
| `expect_log_line` | The matching console line, or null |
| `downtime_seconds` | From the stop to the end of the checks |
| `duration_seconds` | The whole job |

A second deploy request while one is queued or running returns the same job
with `reused: true`. The owner equivalent is `POST /api/v1/servers/{id}/deploy`.
See [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).

## Jobs

### Submitting

Operations that take time return `{"job_id": "...", "reused": false}`. They
include every `server_action`, `restore_backup`, `install_server_marketplace_content`,
`change_native_runtime`, `deploy_pending_changes`, `transfer_server_content`,
and the Valheim jobs. `reused: true` means the same operation was already
queued or running for that server, so you got that job.

Jobs for the same server wait in line instead of failing. While a job waits,
its `stage` says what it waits for, for example *Waiting for a deploy*. Helix
refuses new jobs while a host reboot is scheduled, and when 64 jobs are active
at once.

### Reading

`{"operation": "job_status", "job_id": "..."}` returns the job record:

```json
{
  "id": "0b9a6c7e-5d0f-4f7e-8f0e-2f6d2b1c9a41",
  "kind": "server_deploy",
  "status": "running",
  "stage": "Waiting until the game is ready",
  "progress_percent": 75,
  "created_at_unix_ms": 1761100000000,
  "updated_at_unix_ms": 1761100042000,
  "result": null,
  "error": null
}
```

`status` is `queued`, `running`, `complete` or `failed`. Only `complete` is
success. A failed job still answers with HTTP `200`; read `error`.

`GET /api/v1/automation/jobs/{job_id}/wait?timeout_seconds=12` returns the same
record plus `finished`. It returns as soon as the job is `complete` or
`failed` (`"finished": true`), or after `timeout_seconds` (`"finished": false`).
`timeout_seconds` defaults to 12 and is capped at 12 so the call stays inside
the gateway's 15-second timeout. Call it again to keep waiting. Helix checks
the job every half second. Four waits can be in progress at once.

> [!NOTE]
> The wait route needs Helix 1.10.1 or later. In 1.10.0 the bundled gateway
> answered it with `403 authorization_denied`; poll with `job_status` on that
> version.

`GET /api/v1/automation/jobs` lists the last 256 jobs started with this token:

```json
{"jobs": [{"job_id": "0b9a6c7e-…", "server_id": "helix:EXACT-UUID"}]}
```

A token sees only its own jobs. `job_status` for another job is refused and
logged as `api_token.unsupported`. Job links survive dashboard restarts, and
job records survive broker restarts; a job that was running when the broker
stopped is reported as `failed` with the stage it reached.

Revoking or rotating a token does not cancel its jobs.

## Errors

Errors use the standard shape, `{"code": "...", "message": "..."}`.

| Status and code | Cause |
| --- | --- |
| `401 authentication_required` | No token, a malformed `Authorization` header, more than one of them, or a token that is unknown, expired, revoked, or invalidated by a password or account change |
| `403 authorization_denied` | A `Cookie` or `Origin` header was sent; the server is not on the token; the token lacks the permission; the operation is not available to tokens; the job belongs to another token |
| `400 invalid_json` | Unknown operation or field, a missing required field, or a wrong type |
| `413 payload_too_large` | The body is over 5 MiB |
| `429 token_rate_limited` | More than 600 requests in a minute for this token. `Retry-After: 60` (older versions answered `too_many_attempts` with `Retry-After: 900`) |
| `503 request_capacity_exhausted` | No automation slot was free. `Retry-After: 1` |
| `503 host_broker_unavailable` | The host broker is not answering. `Retry-After: 2` |
| `400 broker_operation_rejected` | Helix accepted the request but the operation was refused. `message` says why |

Common `broker_operation_rejected` messages:

| Message | What to do |
| --- | --- |
| *file revision conflict; reread the file and reconcile before changing or downloading it* | `stat` or `read` again and decide |
| *destination exists; read its revision and explicitly request replacement* | Pass the current revision to replace it |
| *a change is already staged for PATH; pass its revision pending:… to replace it* | Use `pending_list`, then pass the `pending:` revision |
| *file revision conflict; the staged change was replaced or installed. List pending changes or reread the file* | The `pending:` revision is gone; read again |
| *moving files needs the server stopped; …* | Stop the server, or upload under the new path and trash the old one |
| *upload_file is limited to 3 MiB; use chunked upload for larger files* | Use `upload_begin` |
| *upload_file SHA-256 does not match; the destination was not changed* | Hash the exact bytes you send |
| *upload size or SHA-256 does not match; the destination was not changed* | A chunk was wrong or missing; start the upload again |
| *upload is not active for this server; it may have expired or the broker restarted* | Start again with `upload_begin` |
| *two uploads are already active; finish or abort one first* | Wait, or `upload_abort` an old one |
| *text reads are limited to 1 MiB; use chunked download for larger or binary files* | Use `download` |
| *500 changes are already waiting for this server; …* | Deploy or restart first |
| *nothing is staged for this server; upload or write files first* | Nothing to deploy |
| *… is still running on this server, so … was not started. Try again when it finishes.* | Another operation holds the server |
| *a host reboot is scheduled; new background jobs are temporarily unavailable* | Wait for the reboot |
| *console command must be one non-empty line under 512 bytes* | Send one short line |
| *start the server before using its console* | The server is stopped |
| *deep management is available only for Helix-owned servers; …* | The server is an imported AMP server |

A lost response is an unknown outcome. Do not repeat a restart, restore, update
or deploy to find out whether it worked. List the token's jobs and check the
server first.

## Limits

| Limit | Value |
| --- | --- |
| Requests per token | 600 per minute |
| Automation requests in progress | 2 across all tokens; further requests wait for a slot |
| Job waits in progress | 4 |
| Request body | 5 MiB (the gateway allows 12 MiB) |
| `upload_file` | 3 MiB decoded |
| Chunked upload | 0 bytes to 8 GiB, chunks up to 1 MiB, 2 active uploads, 10 minutes idle |
| `read` and `write` | 1 MiB of UTF-8 text |
| `download` chunk | 1 MiB |
| `list` page | 1 to 200 entries |
| Staged changes | 500 per server |
| Token jobs listed | 256 |
| Wait per call | 12 seconds |

A request that waits for a slot still counts against the gateway's 15-second
timeout, so keep scripts sequential.

## Audit log

Each request writes at least two audit records: the permission decision
(`api_token.<permission>`, for example `api_token.files.write`, with `success`
or `denied`) and the result (`api_token.result`, with `success` or `error`).
Transfers write `api_token.transfer`; refused operations write
`api_token.unsupported`. Records name the token and server. They never contain
the token, file contents, or console command text. `GET /api/v1/automation/jobs`
and the wait route do not write audit records.

## Example: deploy a plugin JAR

This example uploads `MyPlugin.jar` to `plugins/` and deploys it, waiting for
the plugin's enable line. It works on a running or a stopped server. The token
needs `files.read`, `files.write` and `restart` on the server.

First load the token and address into the environment as shown in
[Use a token from a script](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens#use-a-token-from-a-script).

### curl

Needs `curl` 7.76 or later, `jq` 1.6 or later, and GNU coreutils.

```bash
#!/usr/bin/env bash
set -euo pipefail
SERVER="helix:EXACT-UUID"
JAR="build/libs/MyPlugin.jar"
DEST="plugins/MyPlugin.jar"

api() {
  curl -sS --fail-with-body "$HELIX_URL/api/v1/automation/server" \
    -H "Authorization: Bearer $HELIX_SERVER_TOKEN" \
    -H "Content-Type: application/json" -d @-
}
files() {  # files '{"action":"stat","path":"..."}'
  jq -n --arg s "$SERVER" --argjson r "$1" \
    '{operation:"server_files", instance_id:$s, request:$r}' | api
}

# 1. Find the revision to replace: a staged change first, then the live file.
REV=$(files '{"action":"pending_list"}' |
  jq -r --arg p "$DEST" '[.changes[] | select(.path==$p and .kind=="write") | .revision][0] // empty')
if [ -z "$REV" ]; then
  FOUND=$(files '{"action":"list","path":"plugins","limit":200}' |
    jq -r --arg p "$DEST" '[.entries[] | select(.path==$p) | .path][0] // empty')
  if [ -n "$FOUND" ]; then
    REV=$(files "$(jq -n --arg p "$DEST" '{action:"stat", path:$p}')" | jq -r .revision)
  fi
fi
# (A folder with more than 200 entries needs next_cursor paging.)

# 2. Upload in one request (up to 3 MiB).
SHA=$(sha256sum "$JAR" | cut -d' ' -f1)
jq -n --arg s "$SERVER" --arg p "$DEST" --arg h "$SHA" --arg r "$REV" \
      --rawfile d <(base64 -w0 "$JAR") \
  '{operation:"server_files", instance_id:$s,
    request:{action:"upload_file", path:$p, data_base64:$d, sha256:$h,
             expected_revision:(if $r=="" then null else $r end)}}' | api | jq .

# 3. Deploy and wait.
JOB=$(jq -n --arg s "$SERVER" \
  '{operation:"deploy_pending_changes", instance_id:$s,
    options:{expect_log:"Enabling MyPlugin"}}' | api | jq -r .job_id)

while :; do
  JOB_JSON=$(curl -sS "$HELIX_URL/api/v1/automation/jobs/$JOB/wait?timeout_seconds=12" \
    -H "Authorization: Bearer $HELIX_SERVER_TOKEN" || true)
  if ! jq -e .status >/dev/null 2>&1 <<<"$JOB_JSON"; then
    # The wait route was refused; poll job_status instead.
    JOB_JSON=$(jq -n --arg j "$JOB" '{operation:"job_status", job_id:$j}' | api)
    sleep 2
  fi
  case $(jq -r .status <<<"$JOB_JSON") in
    complete) jq .result <<<"$JOB_JSON"; break ;;
    failed)   jq -r .error <<<"$JOB_JSON" >&2; exit 1 ;;
  esac
done
```

For a JAR over 3 MiB, replace step 2 with the chunked upload shown in the
Python example.

### Python

Uses the `requests` package. `trust_env = False` stops `requests` from using
proxy settings and `.netrc` credentials from the environment.

```python
import base64
import hashlib
import os
import time

import requests

URL = os.environ["HELIX_URL"].rstrip("/")
SERVER = "helix:EXACT-UUID"
JAR = "build/libs/MyPlugin.jar"
DEST = "plugins/MyPlugin.jar"
DIRECT_LIMIT = 3 * 1024 * 1024

http = requests.Session()
http.trust_env = False
http.headers["Authorization"] = "Bearer " + os.environ["HELIX_SERVER_TOKEN"]


class HelixError(Exception):
    pass


def call(operation, **fields):
    response = http.post(f"{URL}/api/v1/automation/server",
                         json={"operation": operation, **fields},
                         timeout=30, allow_redirects=False)
    if response.status_code != 200:
        try:
            problem = response.json()
        except ValueError:
            problem = {}
        raise HelixError(f"{operation}: HTTP {response.status_code} "
                         f"{problem.get('code')}: {problem.get('message')}")
    return response.json()


def files(action, **fields):
    return call("server_files", instance_id=SERVER, request={"action": action, **fields})


def current_revision(path):
    """The revision to pass when replacing path, or None for a new file."""
    for change in files("pending_list")["changes"]:
        if change["path"] == path and change["kind"] == "write":
            return change["revision"]
    folder, _, _ = path.rpartition("/")
    cursor = None
    while True:
        page = files("list", path=folder, limit=200, cursor=cursor)
        if any(entry["path"] == path for entry in page["entries"]):
            return files("stat", path=path)["revision"]
        cursor = page["next_cursor"]
        if cursor is None:
            return None


def b64(data):
    return base64.b64encode(data).decode("ascii")


def upload(local, remote):
    with open(local, "rb") as handle:
        data = handle.read()
    digest = hashlib.sha256(data).hexdigest()
    expected = current_revision(remote)
    if len(data) <= DIRECT_LIMIT:
        return files("upload_file", path=remote, data_base64=b64(data),
                     sha256=digest, expected_revision=expected)
    started = files("upload_begin", path=remote, size=len(data),
                    sha256=digest, expected_revision=expected)
    if started["completed"]:
        return started  # The destination already has this content.
    upload_id = started["upload_id"]
    offset = started["bytes_written"]
    step = started["max_chunk_bytes"]
    while offset < len(data):
        chunk = data[offset:offset + step]
        offset = files("upload_chunk", upload_id=upload_id, offset=offset,
                       data_base64=b64(chunk))["bytes_written"]
    return files("upload_finish", upload_id=upload_id)


def wait(job_id, deadline_seconds=900):
    deadline = time.monotonic() + deadline_seconds
    while time.monotonic() < deadline:
        response = http.get(f"{URL}/api/v1/automation/jobs/{job_id}/wait",
                            params={"timeout_seconds": 12},
                            timeout=20, allow_redirects=False)
        if response.status_code == 200:
            job = response.json()
        else:
            # The wait route is unavailable here; poll instead.
            job = call("job_status", job_id=job_id)
            if job["status"] in ("queued", "running"):
                time.sleep(2)
        if job["status"] == "complete":
            return job["result"]
        if job["status"] == "failed":
            raise HelixError(job["error"])
    raise HelixError(f"job {job_id} is still running; it was not cancelled")


print(upload(JAR, DEST))
job = call("deploy_pending_changes", instance_id=SERVER,
           options={"expect_log": "Enabling MyPlugin"})
result = wait(job["job_id"])
print("backup:", result["backup"], "downtime:", result["downtime_seconds"], "s")
print("plugin said:", result["expect_log_line"])
```

`upload` reads the whole file into memory, which is fine for plugin JARs. For
very large files, read and hash in chunks instead.

### Python, with the repository client

The [reference client](https://github.com/Riqqqque/Helix/blob/main/examples/integrations/helix_client.py)
uses only the standard library and handles hashing, chunking and safe upload
retries. For tokens it allows plain HTTP only on loopback, so use HTTPS or an
SSH tunnel.

```python
import os
from helix_client import HelixTokenClient

client = HelixTokenClient("http://127.0.0.1:3100", os.environ["HELIX_SERVER_TOKEN"])
server = "helix:EXACT-UUID"
try:
    # Pass expected_revision=... to replace an existing file on purpose.
    client.upload_file(server, "build/libs/MyPlugin.jar", "plugins/MyPlugin.jar")
    job = client.execute("deploy_pending_changes", instance_id=server,
                         options={"expect_log": "Enabling MyPlugin"})
    print(client.wait_for_job(job["job_id"], timeout=900)["result"])
finally:
    client.close()
```

`HelixTokenClient.wait_for_job` uses the wait route, and `deploy(server_id, expect_log=...)` starts a deploy job.

### PowerShell

Works in Windows PowerShell 5.1 and PowerShell 7.

```powershell
$ErrorActionPreference = 'Stop'
$server  = 'helix:EXACT-UUID'
$jar     = 'build\libs\MyPlugin.jar'
$dest    = 'plugins/MyPlugin.jar'
$headers = @{ Authorization = "Bearer $env:HELIX_SERVER_TOKEN" }

function Invoke-Helix([hashtable]$Body) {
    $json = $Body | ConvertTo-Json -Depth 8 -Compress
    Invoke-RestMethod -Method Post -Uri "$env:HELIX_URL/api/v1/automation/server" `
        -Headers $headers -ContentType 'application/json' -Body $json
}
function Invoke-HelixFiles([hashtable]$Request) {
    Invoke-Helix @{ operation = 'server_files'; instance_id = $server; request = $Request }
}

# 1. Revision to replace: a staged change first, then the live file.
$rev = $null
$staged = (Invoke-HelixFiles @{ action = 'pending_list' }).changes |
    Where-Object { $_.path -eq $dest -and $_.kind -eq 'write' }
if ($staged) { $rev = $staged[0].revision }
else {
    $page = Invoke-HelixFiles @{ action = 'list'; path = 'plugins'; limit = 200 }
    if ($page.entries | Where-Object { $_.path -eq $dest }) {
        $rev = (Invoke-HelixFiles @{ action = 'stat'; path = $dest }).revision
    }
}

# 2. Upload: one request up to 3 MiB, chunks above that.
$bytes = [IO.File]::ReadAllBytes((Resolve-Path $jar))
$sha   = (Get-FileHash -Algorithm SHA256 -Path $jar).Hash.ToLowerInvariant()
if ($bytes.Length -le 3MB) {
    Invoke-HelixFiles @{ action = 'upload_file'; path = $dest; sha256 = $sha
        data_base64 = [Convert]::ToBase64String($bytes); expected_revision = $rev } | Out-Null
} else {
    $begin = Invoke-HelixFiles @{ action = 'upload_begin'; path = $dest
        size = $bytes.Length; sha256 = $sha; expected_revision = $rev }
    if (-not $begin.completed) {
        $offset = [long]$begin.bytes_written
        while ($offset -lt $bytes.Length) {
            $count = [Math]::Min([long]$begin.max_chunk_bytes, $bytes.Length - $offset)
            $chunk = [Convert]::ToBase64String($bytes, [int]$offset, [int]$count)
            $offset = [long](Invoke-HelixFiles @{ action = 'upload_chunk'
                upload_id = $begin.upload_id; offset = $offset; data_base64 = $chunk }).bytes_written
        }
        Invoke-HelixFiles @{ action = 'upload_finish'; upload_id = $begin.upload_id } | Out-Null
    }
}

# 3. Deploy and wait.
$job = Invoke-Helix @{ operation = 'deploy_pending_changes'; instance_id = $server
    options = @{ expect_log = 'Enabling MyPlugin' } }
while ($true) {
    try {
        $status = Invoke-RestMethod -Method Get -Headers $headers `
            -Uri "$env:HELIX_URL/api/v1/automation/jobs/$($job.job_id)/wait?timeout_seconds=12"
    } catch {
        # The wait route was refused; poll job_status instead.
        $status = Invoke-Helix @{ operation = 'job_status'; job_id = $job.job_id }
        if ($status.status -in 'queued', 'running') { Start-Sleep -Seconds 2 }
    }
    if ($status.status -eq 'complete') { $status.result | ConvertTo-Json -Depth 8; break }
    if ($status.status -eq 'failed') { throw $status.error }
}
```

Windows PowerShell 5.1 sends a string body as ISO-8859-1, so keep `expect_log`
and file paths to plain ASCII there, or use PowerShell 7. When a call fails,
`Invoke-RestMethod` throws. The Helix problem JSON is in
`$_.ErrorDetails.Message`.

## More examples

Restart a server and wait:

```bash
curl -sS "$HELIX_URL/api/v1/automation/server" \
  -H "Authorization: Bearer $HELIX_SERVER_TOKEN" -H "Content-Type: application/json" \
  -d '{"operation":"server_action","instance_id":"helix:EXACT-UUID","action":"restart"}'
```

Read the last 200 console lines:

```bash
curl -sS "$HELIX_URL/api/v1/automation/server" \
  -H "Authorization: Bearer $HELIX_SERVER_TOKEN" -H "Content-Type: application/json" \
  -d '{"operation":"server_logs","instance_id":"helix:EXACT-UUID","lines":200}' | jq -r '.lines[]'
```

Run a console command:

```bash
curl -sS "$HELIX_URL/api/v1/automation/server" \
  -H "Authorization: Bearer $HELIX_SERVER_TOKEN" -H "Content-Type: application/json" \
  -d '{"operation":"server_console","instance_id":"helix:EXACT-UUID","command":"list"}'
```

Edit a config file on a running server, then deploy:

```json
{"operation":"server_files","instance_id":"helix:EXACT-UUID","request":{"action":"read","path":"plugins/MyPlugin/config.yml"}}
```

```json
{"operation":"server_files","instance_id":"helix:EXACT-UUID","request":{"action":"write","path":"plugins/MyPlugin/config.yml","content":"enabled: true\n","expected_revision":"REVISION-FROM-READ"}}
```

```json
{"operation":"deploy_pending_changes","instance_id":"helix:EXACT-UUID"}
```

Replace a versioned JAR (`MyPlugin-1.2.jar` with `MyPlugin-1.3.jar`) on a
running server: upload the new file, trash the old one with its revision, then
deploy. Both changes are staged and installed together.

Set keep rules and empty the backup trash after 14 days:

```json
{"operation":"set_backup_policy","instance_id":"helix:EXACT-UUID","keep_count":10,"keep_days":30,"trash_days":14}
```

Empty the whole backup trash now:

```json
{"operation":"purge_backup_trash_older_than","instance_id":"helix:EXACT-UUID","days":0}
```

Preview copying plugins and configs from a test server to production:

```json
{"operation":"server_transfer_preflight","instance_id":"helix:TEST-UUID","target_id":"helix:PROD-UUID","parts":["plugins","configs"]}
```

## Related pages

- [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)
- [API and Integrations](https://github.com/Riqqqque/Helix/wiki/API-and-Integrations)
- [Dashboard API Reference](https://github.com/Riqqqque/Helix/wiki/Dashboard-API-Reference)
- [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes)
- [Backups](https://github.com/Riqqqque/Helix/wiki/Backups)
- [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers)
