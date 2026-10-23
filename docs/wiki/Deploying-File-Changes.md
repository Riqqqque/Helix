# Deploying File Changes

You can upload, write, or delete server files through the server API while the
server is running. Helix does not touch the live server folder when you do.
It stages each change and installs them all at once the next time the server
starts, restarts, or runs a **Deploy**. This page explains how staging works and
how a deploy installs changes safely.

Staging applies to the server file API: the dashboard route
`POST /api/v1/servers/{instance_id}/files` and the token operation
`server_files`. The **Files** tab on the server page is the host file browser and
writes the folder directly. Edit files there only while the server is stopped.

## What happens when you change a file

| Server state | Write, create, upload, or trash (delete) | Move |
| --- | --- | --- |
| Stopped | Changes the live folder right away | Works |
| Running | Staged; the live folder is not changed | Refused |

When a change is staged, the response says so:

```json
{
  "path": "plugins/MyPlugin.jar",
  "pending": true,
  "kind": "write",
  "size": 482113,
  "sha256": "…",
  "revision": "pending:3f1c…",
  "applies": "on the next start, restart, or deploy"
}
```

Staged files are kept in Helix's own state folder
(`<state_root>/pending/<server-id>`, owned by the broker), not in the server
folder. The game cannot see or change them, and nothing it reads changes until
the change is installed.

Some details:

- **Uploads.** Small files use `upload_file` (up to 3 MiB). Larger files use the
  chunked upload steps (up to 8 GiB in 1 MiB chunks). An upload started while the
  server was stopped but finished after it started is staged too.
- **Folders.** Missing parent folders are created when the change is installed.
- **Same file.** If the server already has exactly the file you upload (same
  size and SHA-256), nothing is staged and any change waiting for that path is
  dropped.
- **Delete.** Trashing a file that exists is staged as a delete. Trashing a file
  that exists only as a staged new file simply drops the staged change.
- **Move.** Moving needs a stopped server. On a running server, upload the file
  to its new path and trash the old one; both are staged.
- **Limit.** Up to 500 changes can wait for one server. Restart it or run a
  deploy to install them first.
- **Stopped server wins.** A direct change to a stopped server replaces anything
  staged for the same path.

## Revisions and conflicts

File changes use revisions so you never overwrite something by accident.

- To replace an existing live file, pass its current `revision` from `stat` or
  `read` as `expected_revision`.
- To change a file you already staged, pass the staged change's `pending:…`
  token as `expected_revision`.
- If you write to a path that already has a staged change without its token, the
  request fails and names the token to use: `a change is already staged for
  plugins/MyPlugin.jar; pass its revision pending:… to replace it`.
- If the token no longer matches (the change was replaced, discarded, or already
  installed), the request fails with `file revision conflict; the staged change
  was replaced or installed`. List the pending changes or read the file again.
- A new file needs no revision, but fails if the path already exists on the live
  server.

The live file can still change after you stage a change, for example when a
plugin rewrites its config on shutdown. At install time the staged version wins.
The file it replaced goes to the Files trash (`.helix-trash`), and the install
result marks that entry `changed_since_staged: true`.

## Seeing and discarding staged changes

When changes are waiting, the server page shows a banner: **N file changes
waiting for a restart**.

- **Show** lists each path with its size and whether it is new or replaces the
  current file. The **x** next to an entry discards that one change.
- **Discard all** drops every staged change.
- **Deploy** opens the deploy dialog (below).

Through the API:

```json
{"action":"pending_list"}
```

returns `changes` (each with `path`, `kind`, `size`, `sha256`, `revision`,
`replaces_existing`, and `staged_at_unix_ms`), plus `count` and `total_bytes`.

```json
{"action":"pending_discard","path":"plugins/MyPlugin.jar"}
```

discards one change. Leave out `path` to discard everything. For tokens,
`pending_list` needs `files.read` and `pending_discard` needs `files.write`.

Restoring a backup discards all staged changes for that server, because they
were made against files the restore replaces.

## Installing on Start and Restart

**Start** on a stopped server and **Restart** on any server install every staged
change before the server starts. Replaced and deleted files go to the Files trash.

Installation is all or nothing. If any change cannot be installed (for example a
staged file is missing or fails its checksum), Helix puts back the changes it had
already installed and leaves every change staged. The server still starts on its
current files, and the action result reports the reason in `pending_error`. A
successful install is listed in `applied_pending`.

Start and Restart do not take a backup and do not roll back if the server fails
to come up with the new files. Use Deploy for that.

## The Deploy job

Deploy installs staged changes as one job and checks that the server comes back.
Start it from the banner, with `POST /api/v1/servers/{instance_id}/deploy`
(`games.manage`), or with the token operation `deploy_pending_changes` (the
token needs the `restart` permission).

The job runs these steps:

1. **Wait its turn.** If another job holds the server, the deploy waits in line
   and its stage says what it is waiting for.
2. **Back up** (option `backup`, on by default). A running Java Minecraft server
   is backed up live while players stay online. Other games, Pumpkin, and a
   Minecraft server whose console does not answer are stopped for the backup. If
   the backup fails, nothing is deployed. See
   [Backups](https://github.com/Riqqqque/Helix/wiki/Backups).
3. **Stop** the server, if it is still running.
4. **Install** every staged change. If one fails, the others are put back, the
   server is started again if it was running, and the job fails with the reason.
   The changes stay staged.
5. **Start** the server (option `start`, on by default; a server that was
   running is always started again).
6. **Wait until ready.** Minecraft must answer a status ping, which happens after
   Paper prints its `Done` line. Pumpkin must also answer on its Bedrock port.
   Other games must write their ready marker. Helix also checks that saved
   Minecraft settings were not overwritten during startup.
7. **Check the console** (optional `expect_log`). Helix waits up to a minute for a
   console line containing that text, such as a plugin's enable message. The text
   is 1–200 characters.

If step 5, 6, or 7 fails and `revert_on_failure` is on (the default), Helix
rolls back:

- It stops the server and puts the previous files back from the Files trash. The
  versions you deployed stay in the Files trash.
- It starts the server again if it was running before, so it runs the previous
  version.
- The job fails with a message that says what went wrong, whether the files were
  put back, and the ID of the backup taken in step 2.

With `revert_on_failure` off, a failed check leaves the new files in place and
the message says nothing was put back.

The dashboard dialog offers **Back up first** and **Wait for this console line**.
`start` and `revert_on_failure` are API options; both default to on.

### Deploy options

| Option | Default | Meaning |
| --- | --- | --- |
| `backup` | `true` | Back up before installing |
| `start` | `true` | Start the server afterwards even if it was stopped before |
| `expect_log` | none | Console text that proves the deploy worked |
| `revert_on_failure` | `true` | Put the previous files back and start again if the server does not come up |

### Result

A finished deploy returns:

| Field | Meaning |
| --- | --- |
| `deployed` | `true` |
| `applied` | Each installed change: `path`, `kind`, `recovery_path` (where the replaced file went in the Files trash), `changed_since_staged` |
| `backup` | `backup_id` and `mode` (`live`, `stopped`, or `offline`), or `null` when `backup` was off |
| `online` | Whether the server was started |
| `expect_log_line` | The console line that matched `expect_log`, or `null` |
| `downtime_seconds` | From the stop until the server was ready (and the console line seen) |
| `duration_seconds` | The whole job, including waiting and the backup |

Because a live backup runs before the stop, downtime only covers stop, install,
and start. When the server had to stop for its backup, `downtime_seconds` starts
after the backup, so the real outage is longer by the backup time.

As a measured example, a staged deploy on one test server had 21
seconds of downtime. Your number depends on the game, the world, and the plugins.

A deploy with nothing staged fails right away with `nothing is staged for this
server`.

## Worked example: deploy a plugin JAR

This example replaces a plugin on a running Paper server with a
[server API token](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens) that
has `files.read`, `files.write`, and `restart` on that server. Every request goes
to `POST /api/v1/automation/server` with `Authorization: Bearer <token>`. The
[Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API) page has
the request details.

1. Read the current file's revision:

   ```json
   {"operation":"server_files","instance_id":"helix:SERVER-UUID","request":{"action":"stat","path":"plugins/MyPlugin.jar"}}
   ```

2. Upload the new JAR over it. For a file up to 3 MiB, send it in one request
   with its SHA-256 and the revision from step 1:

   ```json
   {"operation":"server_files","instance_id":"helix:SERVER-UUID","request":{"action":"upload_file","path":"plugins/MyPlugin.jar","data_base64":"UEsDB…","sha256":"SHA256-OF-THE-FILE","expected_revision":"REVISION-FROM-STAT"}}
   ```

   The server keeps running. The response has `"pending": true`.

3. Check what is waiting:

   ```json
   {"operation":"server_files","instance_id":"helix:SERVER-UUID","request":{"action":"pending_list"}}
   ```

4. Deploy, and wait for the plugin's enable line:

   ```json
   {"operation":"deploy_pending_changes","instance_id":"helix:SERVER-UUID","options":{"expect_log":"Enabling MyPlugin"}}
   ```

   The response has a `job_id`.

5. Wait for the job:

   ```http
   GET /api/v1/automation/jobs/JOB-ID/wait?timeout_seconds=12
   ```

   Call it again while `finished` is `false`. When the job completes, its result
   shows the installed change, the backup ID, and `downtime_seconds`.

If MyPlugin fails to enable, the deploy puts the old JAR back, starts the server
on it, and reports the backup ID. The new JAR stays in `.helix-trash` so you can
inspect it.

To install several files at once, stage all of them first and run one deploy.
