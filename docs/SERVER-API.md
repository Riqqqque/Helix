# Server automation API

## Modpack release notes

`GET /api/v1/servers/minecraft/modpacks/projects/{project_id}/versions/{version_id}/changelog?provider=modrinth`
requires `games.view`; `provider=curseforge` uses the protected server-side API key.
The release must belong to the requested project. The response contains `body`,
`format` (`markdown` or `html`) and `truncated`, with at most 100,000 characters.
The update notice loads notes only when Changelog is opened, using the exact
offered release, not the catalog's latest release. Viewing notes does not update
or restart the server. The UI renders catalog content through the existing safe
marketplace renderer rather than inserting raw HTML.

Valheim also has a typed [configuration and mod-management API](VALHEIM.md#api).
Check `valheim_management` in a native server's capabilities before using it.

## Scheduled restarts

Native servers expose `restart_schedule` in their detail response. Native and
imported AMP games also expose `GET /api/v1/servers/{instance_id}/restart-schedule`
to browser sessions with `games.view`. Sessions with `games.manage` and CSRF proof can use
`PUT /api/v1/servers/{instance_id}/restart-schedule` with
`{"first_at_unix_ms": 1800000000000, "interval_hours": 24}`. Send JSON `null` to
disable, including cancelling an active warning countdown. This operation is
deliberately not granted to scoped automation tokens: a temporary token should
not silently leave permanent restart authority behind after it expires.

Schedules start disabled. API timestamps must be 6 minutes to 366 days ahead;
intervals are 6, 12, 24, 48 or 168 elapsed hours. The dashboard only asks for an
interval and a clock time, defaulting to every 24 hours at 5 AM in the browser's
timezone. It computes the next occurrence automatically, with one extra minute
of request headroom beyond the warning window. Passed or too-close times advance
to the next safe occurrence, including across midnight. New intervals of 24 hours
or longer start at the next eligible clock time, then repeat at that interval.
Editing an unchanged schedule preserves its existing future anchor. A live preview
shows the next restart. Fixed-hour recurrence does not preserve local wall-clock
time across daylight saving changes.

The broker runs schedules without an open dashboard. Private atomic records keep
the next occurrence, warning phase, runtime identity and last outcome. Warnings
are sent at roughly 5 minutes, 1 minute, 10 seconds and shutdown. Missed warning
phases, failed console delivery, stopped/replaced servers, conflicting jobs and
pending host reboots skip the occurrence. Missed runs never catch up. Broker
restart abandons an in-progress occurrence rather than replaying it. Disabling is
blocked once a restart has been claimed; the UI displays that executing state.

Execution uses the existing per-server operation lock and job registry. Java
Minecraft requires positive `save-all flush` confirmation, requests SIGTERM with Docker's
infinite stop grace period (no SIGKILL fallback), then verifies stopped/non-OOM
state and a normal or SIGTERM exit. A 180-second client timeout stops waiting,
not the game. Hung shutdowns require operator attention. Saved server properties
are preserved, and startup readiness and settings checks must pass. This is not
a substitute for backups and cannot guarantee correct persistence inside every
third-party mod. Neither a failed startup nor an interrupted run is auto-retried.

Read `state`, `next_at_unix_ms`, `interval_hours`, `last_result` and
`last_at_unix_ms` for status. States are `disabled`, `scheduled`, `warning`,
`restarting` and `unavailable`; unavailable or corrupt records never authorize a
restart. Status also includes `player_warnings`, `shutdown_method` and
`runtime_update_required`.

All current native game types have an explicit shutdown policy. Java Minecraft
uses the confirmed flush described above. Pumpkin saves during graceful shutdown;
its asynchronous save command is not treated as proof of a completed flush.
Valheim uses Ctrl+C, Terraria/tModLoader uses console `exit`, and V Rising uses a
Windows console Ctrl+C helper. Non-Minecraft servers require runtime version 2;
older runtimes cannot schedule restarts. Upgrade them while stopped, preserving
their existing data. Actual persistence still depends on the game and its mods.

Imported AMP games use the registered instance's Core Stop and Start methods,
with running/stopped state checks and a runtime identity derived from uptime.
The ADS controller itself is excluded. AMP controls its own shutdown policy;
Helix does not override AMP timeouts or guarantee that a module never force-stops.
AMP integration and native schedule storage must both be configured on the broker.

Non-Minecraft and imported AMP schedules do not claim in-game warnings. Enabling
one requires `"allow_unwarned_restart": true` in the request and explicit consent
in the UI. They still use the five-minute identity-check countdown and skip
missed phases. Unknown future native game types must add an explicit policy at
compile time instead of inheriting generic restart behavior.

Start with [authentication and the Python client](INTEGRATIONS.md), then read
`GET /api/v1/servers`. Keep the exact returned `id`, including its manager prefix.
`GET /api/v1/servers/{instance_id}/capabilities` reports the operations available
for that game and manager. Permissions still belong to the logged-in account;
these routes do not create per-server credentials. For restricted headless
access, use [server tokens](SERVER-TOKENS.md) through the automation endpoint.

The [OpenAPI document](openapi.json) lists every current server route and method,
including creation, ports, artwork, console, settings, marketplace, runtime,
backups, removal and jobs. Request bodies are typed. Some broker responses remain
open objects, so this is not yet complete generated-client coverage.

## Game and manager coverage

| Operation | Native Minecraft, including Pumpkin/custom | Native V Rising, Valheim, Terraria | Imported AMP |
| --- | --- | --- | --- |
| Start, stop, restart | Yes | Yes | Adapter actions |
| Force-stop (`kill`) | Yes | Yes | Not supported; use AMP |
| Files and transfers | Yes | Yes | Use AMP or migrate a stopped copy |
| Logs and retained console history | Yes | Yes | Use AMP |
| Console commands | Minecraft RCON | No command-console adapter | Use AMP |
| Settings form | Minecraft settings | Edit the game's config files | Use AMP |
| Backups, restore, trash, export | Yes | Yes | Adapter backup creation; other controls in AMP |
| Software update | Published software/modpacks; custom JARs use Files | Existing game-specific update process | Adapter action |
| Exact runtime repair/version choice | Published Minecraft software | Not supported | Use AMP |
| Startup, memory, CPU, host ports, removal | Yes | Yes | Use AMP |

Marketplace content must match the selected game/loader. Pumpkin is not Paper.
Custom software has no publisher release feed; replace its artifact through
Files after a backup. A capabilities response describes the adapter, not proof
that its remote service is currently reachable. Check job completion and logs.

## Start, stop, kill, update and backup

`POST /api/v1/servers/{instance_id}/actions` accepts
`{"action":"start"}`, `stop`, `restart`, `kill`, `update` or `backup`.
Native actions return a `job_id`: poll `/api/v1/jobs/{job_id}` until `complete`
or `failed`. Prefer stop; kill can lose unsaved game progress. Software updates
have their own compatibility checks and backup/rollback workflow. Do not bypass
them by editing Helix's private registry.

A lost response is an unknown outcome, not permission to repeat a mutation.
Reconcile inventory, jobs and file revisions before continuing. General
idempotency keys and durable upload sessions across broker restarts are not
implemented.

## Server-relative files

Send JSON to `POST /api/v1/servers/{instance_id}/files`. Every operation, even
reads, needs `games.view` and `games.manage`, a session cookie, CSRF proof, and
the configured Origin. Server files can contain RCON and plugin secrets.
No `storage.files.manage` grant or absolute host path is needed.

| Action | Body fields besides `action` | Result/use |
| --- | --- | --- |
| `list` | `path`, `limit` (1–200), optional `cursor` | Page of entries and `next_cursor`; empty path means server root |
| `stat` | `path` | Kind, byte size and opaque `revision` |
| `read` | `path` | UTF-8 text and revision, up to 1 MiB |
| `create` | `path` | New empty file; never overwrites |
| `mkdir` | `path` | One new directory; parent must exist |
| `write` | `path`, `content`, `expected_revision` | Atomic text replacement and `recovery_path` |
| `move` | `path`, `destination`, `expected_revision` | Rename or move a file/folder inside this server; never overwrites |
| `trash` | `path`, `expected_revision` | Recoverable removal and `recovery_path` |
| `download` | `path`, `offset`, `length`, `expected_revision` | Base64 chunk, SHA-256, offsets, size, revision, EOF |
| `upload_begin` | `path`, `size`, `sha256`, optional `expected_revision` | Upload ID and chunk limit |
| `upload_chunk` | `upload_id`, `offset`, `data_base64` | Acknowledged `bytes_written` |
| `upload_status` | `upload_id` | Current offset, target and expected size |
| `upload_finish` | `upload_id` | Verified atomic publication and optional recovery path |
| `upload_abort` | `upload_id` | Discard uncommitted staged bytes |

Paths use `/` and stay relative to the selected server. Parent traversal,
absolute paths, symbolic links, hard-linked files, sockets and devices are
refused. `.helix-api-*` names are reserved. The root and recovery directory
cannot be renamed or removed. Operations use directory descriptors so replacing
a path component with a symbolic link does not redirect access to the host.
Do not concurrently move server directories with external administrator tools.

Create, mkdir, write, move, trash and upload finish require the server to be
stopped, verified against its exact container identity. Upload staging can run
while online; it does not change live server files. Each request participates in
the native operation lock, so a backup/update/restore cannot overlap its file
operation. Stop and wait for completion before publishing replacement files.

Revisions are opaque, not timestamps to invent or parse. Keep the value from
`stat` or `read`. A stale revision rejects the operation; reload and reconcile.
Downloads check revisions before and after each chunk. A live log or world file
may change: stop the server or download a completed backup for a stable copy.
Directory cursors paginate names, not an immutable snapshot; avoid concurrent
changes while enumerating a tree and inspect `omitted_entries`.

## Upload, download and copy

Uploads accept zero bytes through 8 GiB, in sequential chunks of at most 1 MiB.
There are two active upload slots. Inactive sessions expire after ten minutes,
with cleanup on the next upload request. Finish verifies the declared size and
SHA-256 before making the file visible. A destination that already exists needs
its expected revision; accidental overwrite is rejected.

Uploads use Linux unnamed temporary files on the destination filesystem.
Aborting or losing the broker closes them without leaving partial plugin/world
files. Refreshing the browser does not stop them: retain the upload ID and use
`upload_status` to reconcile the last acknowledged offset. A broker restart
invalidates in-flight IDs; start a new upload after checking the destination.
Filesystems must support `O_TMPFILE`, hard links and atomic exchange for
replacement (for example ext4 or XFS). Unsupported filesystems fail before
replacement; Docker writable layers and some network filesystems may not support
these operations. Run server data on a supported host volume.

The reference client exposes:

```python
client.server_capabilities(server_id)
client.iter_server_directory(server_id, "mods")
client.download_file(server_id, "logs/latest.log", "saved-log.txt")
client.upload_file(server_id, "plugin.jar", "plugins/plugin.jar")
client.transfer_file(source_id, "config/example.toml", target_id, "config/example.toml")
```

Upload and transfer default to create-new. Pass `expected_revision=` only after
reading the target and deliberately choosing replacement. Transfers copy through
the client, with bounded memory and two revision-checked reads of the source
(hashing, then transfer). No server-side arbitrary URL fetch is exposed. The
client does not silently stop, restart, delete or overwrite anything. Download
creates a new local file atomically and never overwrites an existing local file.

For a folder, paginate each directory, create destination directories explicitly
and copy the regular files. Do not follow links or assume a partially copied
tree is complete. Recursive archive extraction, arbitrary shell execution,
permission/ownership changes and permanent file deletion are intentionally not
part of this endpoint. Use recoverable trash and the existing backup workflow.

## Undo and backup export

`trash` and successful replacements return a relative `recovery_path` under
`.helix-trash`. Save it alongside the original path. To restore: stat the
recovery path and move it back with that revision. If the original path is
occupied, trash that item first; move never overwrites. File trash is not
automatically purged and consumes disk space. Full server backups remain
separate from per-file recovery.

`POST /api/v1/servers/{instance_id}/backups/{backup_id}/download` needs
`games.backups.manage`. Send `{"action":"stat"}` first, then
`{"action":"download","offset":0,"length":1048576,"expected_revision":"..."}`.
Continue using the returned `next_offset` until EOF. This exports the exact
`.tar.gz` backup archive, not private registry metadata. Keep the backup ID and
software/version notes with your export. Automatic import of arbitrary backup
archives is not provided by this route.

See the existing console, settings, marketplace, runtime, backup-policy and
removal routes in OpenAPI for the rest of the server controls. Host networking
still stops at the host firewall; users configure their router themselves.

## Configuration change notices

Minecraft server detail includes `config_changes` with `state`, `files`, and
`limited`. States are `changed`, `no_changes`, `unknown`, and `stopped`.
The server page shows saved configuration changes with the existing restart
confirmation. It never restarts a game automatically.

Helix keeps private file fingerprints for each Docker start, captured after a
managed start, before a settings save, or when an already-running server is first
viewed. Later edits remain visible across page refreshes and dashboard restarts,
including edits saved by an in-game mod screen while the dashboard is closed.
Writing identical content or reverting an edit does not trigger a notice.

The check covers common root settings, `config`, the selected world's
`serverconfig`, and plugin files named for config/settings. It skips symlinks,
world regions, backup files, and plugin player databases. Each scan is bounded
to 2,048 entries, four directory levels, 512 KiB per file, and 8 MiB total;
the response lists at most 32 changed paths and never returns their contents.
Partial scans report `limited`; unavailable checks report `unknown`.

This is saved-file change detection, not a generic mod reload API. It cannot
observe settings a mod never writes, changes before the first baseline, or
prove that a running mod applied a value. A new server start resets the baseline
after readiness; that is not a claim that every mod accepted its configuration.

## Regression checks

`docker build --target linux-test .` runs the Rust checks plus a real HTTP and
broker smoke test in disposable storage. It covers all six native game/software
fixtures, multi-chunk uploads and downloads, file transfer, revisions, recovery,
backup export and rejected paths. Docker/system-service commands in that fixture
are no-ops; it does not boot games or prove third-party manager availability.
Client and contract checks also run with
`python3 -m unittest discover -s examples/integrations -p 'test_*.py' -v`.
