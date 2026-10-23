# Backups

Every Helix-managed server has full local backups: one `.tar.gz` archive of the
server folder plus a small definition file that lets Helix restore it. This page
covers making backups, what goes into them, restoring, downloading, the keep
rules, and the backup trash.

Backups for imported AMP servers stay in AMP. See
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace#imported-amp-servers).

## Back up now

Open a server, choose the **Backups** tab, then **Back up now**. The backup runs
as a background job, so you can close the page. If the server is busy with
something else, the backup waits in line and the server page shows what it is
waiting for.

How Helix makes the backup depends on the game and on whether the server is
running:

| Server | What happens | Result `mode` |
| --- | --- | --- |
| Running Java Minecraft (Paper, Purpur, Folia, Leaves, Pufferfish, Fabric, Quilt, Forge, NeoForge, Vanilla, modpacks, custom JAR) | Live backup. Players stay connected. | `live` |
| Running Minecraft whose console does not answer | Stop, back up, start again. | `stopped` |
| Running Pumpkin, Hytale, Valheim, V Rising, Terraria, Palworld, and the other games | Stop, back up, start again. | `stopped` |
| Any stopped server | Back up. The server stays stopped. | `offline` |

The job result includes `backup_id`, `mode`, and any backups the keep rules moved
to the trash (`retention_trashed`).

## Live backups for Minecraft

A running Java Minecraft server is backed up without a restart. Helix talks to it
through its private console (RCON on the host's loopback address):

1. `save-off` turns off autosave, so the game stops writing world files.
2. `save-all flush` writes everything in memory to disk. Helix waits up to two
   minutes for the reply.
3. Helix archives the server folder while players keep playing.
4. `save-on` turns autosave back on. Helix always sends it, even if the archive
   failed, and retries up to five times because a busy server can miss one
   request.

Files such as plugin data or logs may change while they are read. In a live
backup that is expected and does not fail the backup; the finished archive is
still checked (see [Compression and checks](#compression-and-checks)).

If `save-off` or `save-all flush` does not get an answer, Helix turns autosave
back on and falls back to the stop path: it stops the server cleanly, archives
it, starts it again, and waits until it is ready. The result `mode` is then
`stopped`.

If the archive finished but Minecraft did not confirm `save-on`, the job fails
with the backup ID and asks you to run `save-on` in the Console. The backup
itself is kept.

Helix does not use live backups for Pumpkin, so a running Pumpkin server always
takes the stop path. So do Hytale, Valheim, and the other non-Minecraft games.

## What is left out

Minecraft backups (including Pumpkin) leave out folders the game recreates on its
own:

| Left out | Why |
| --- | --- |
| `logs/` | Old log files |
| `crash-reports/` | Old crash reports |
| `cache/` | Paper's download cache. Paper downloads it again on the first start after a restore, so that start takes a little longer. |
| `plugins/.paper-remapped/` | Paper's remapped plugin cache, rebuilt on start |
| `.helix-trash/` | The Files trash, where Helix keeps files replaced or deleted through the server file API |

Backups of every other game leave out only `.helix-trash/`. Everything else in
the server folder is included: worlds, plugins, mods, configuration, player data,
and game-specific sign-in files such as Hytale's `auth.enc`.

Only the server's own folder is archived. Other disks mounted inside it are not
followed.

## Compression and checks

When `pigz` is installed on the host (`/usr/bin/pigz`), Helix compresses with it
on half of the CPU cores, at most 8, leaving the rest for the games. Without
`pigz`, Helix uses ordinary single-threaded `gzip`. To get parallel compression
on Debian or Ubuntu:

```bash
sudo apt install pigz
```

Either way the archive is an ordinary `.tar.gz` that `tar` and `gzip` read.

Helix writes the archive to a `.tar.gz.partial` file first. When it is complete,
Helix tests the whole archive (`pigz --test` or `gzip --test`). Only an archive
that passes is renamed into place and listed. A failed or timed-out backup leaves
nothing behind. A single backup may run for up to 30 minutes.

## Where backups live

Backups are stored under the broker's `backup_root`. The one-command installer
sets it to `/srv/helix/backups`:

```text
/srv/helix/backups/
  <server-id>/
    <created-unix-ms>.tar.gz     the archive
    <created-unix-ms>.json       the server definition at backup time
  .trash/
    <server-id>/
      <trash-id>/                one deleted backup
        <created-unix-ms>.tar.gz
        <created-unix-ms>.json
        trash.json               when it was deleted
```

The backup ID is the creation time in Unix milliseconds. The `.json` file next to
each archive is Helix's definition of the server when the backup was made: its
software, version, ports, memory, and the rest of its settings. Restore needs it.
In the Backups list, a backup with a definition shows **Restorable**; an archive
without one shows **Archive only** and cannot be restored from the dashboard.

The folders are readable only by the broker. Backups live on the same disk as the
servers unless you point `backup_root` somewhere else.

## Restore

Choose **Restore** on a backup and confirm. Restore runs as a job that waits in
line behind other work on the server.

1. Helix checks that the backup belongs to this server.
2. A running server is stopped.
3. Helix makes a **safety backup** of the current files. If that fails, nothing
   is restored and the server is started again if it was running.
4. The current folder is moved aside, and the archive is extracted into a fresh
   folder with the server's own file owner.
5. Helix's record of the server (software, version, build, and settings) is taken
   from the backup's definition file.
6. A server that was running is started and Helix waits until it is ready.

If any step after the move fails, Helix puts the previous folder back and starts
the server again if it was running. The safety backup appears in the Backups list
like any other backup.

Restore also discards any staged file changes for the server, because they were
made against files that no longer exist. See
[Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).

## Download a backup

You can copy an archive off the machine through the API. This needs the
`games.backups.manage` permission (or `backups.read` for a
[server API token](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)).

```http
POST /api/v1/servers/{instance_id}/backups/{backup_id}/download
Content-Type: application/json

{"action":"stat"}
```

Then download it in chunks of up to 1 MiB, passing the revision from `stat`:

```json
{"action":"download","offset":0,"length":1048576,"expected_revision":"REVISION"}
```

Repeat with the returned `next_offset` until the end of the file. You get the
exact `.tar.gz`, not the `.json` definition, so note the server's software and
version with your copy. See the
[Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API) for token
requests.

## Keep rules

The **Automatic cleanup** form on the Backups tab sets three rules per server:

| Field | Range | Meaning |
| --- | --- | --- |
| Keep the newest | 0–50 backups | Extra oldest backups move to the trash. 0 means no count limit. |
| Remove backups older than | 0–365 days | Older backups move to the trash. 0 means no age limit. |
| Empty the trash after | 0–365 days | Trashed backups are deleted for good this long after they were deleted. 0 means never. |

With both keep limits set, Helix keeps the newest backups up to the count and
moves anything over the age limit to the trash.

Keep rules run after every backup and when you choose **Save cleanup rules** or
**Apply rules now**. The backup that just finished is never moved. Keep rules
only move backups to the trash; they never delete one.

## The backup trash

Deleting a backup moves it to the server's **Trash**. Nothing is copied: the
archive and its definition are moved with hard links, so it takes no extra space
and no time.

From the Trash list you can:

- **Undo** to put a backup back in the list.
- Delete one backup forever with the bin button. This cannot be undone.
- **Empty trash** to delete every trashed backup for this server, after you
  confirm.

The Trash header shows how many backups it holds and how much disk they use. Each
entry shows when the backup was made and when it was deleted. Helix shows a
warning once a server's trash passes 20 GiB.

### Automatic expiry

Since 1.10.0 the trash empties itself. A trashed backup is deleted for good once
it has been in the trash longer than the server's **Empty the trash after**
setting.

- The default is 7 days. The host-wide default is `backup_trash_retention_days`
  in the broker's `native` settings (0–365); a per-server setting overrides it.
- 0 turns automatic expiry off for that server. Deleted backups then stay until
  you delete them yourself, which is how Helix behaved from 1.4.0 until 1.10.0.
- Helix checks every hour and after each backup.
- Changing the setting applies to backups already in the trash. Shortening it can
  delete older entries on the next check.
- Backups that were already in the trash when you upgraded to 1.10.0 are counted
  from the upgrade, not from when they were deleted, so nothing disappeared right
  after the upgrade.

Emptying a large trash runs separately from the server, so it does not hold up
deploys, server actions, or file access.

### Purge older than N days

To delete trashed backups that were deleted more than N days ago, without waiting
for the automatic check, use the API:

```http
POST /api/v1/servers/{instance_id}/backups/trash/purge
Content-Type: application/json

{"older_than_days": 3}
```

`older_than_days` is 0–3650, and 0 empties the whole trash. This counts from the
actual deletion time, even for entries that were in the trash during the upgrade.
The token operation is `purge_backup_trash_older_than` with the `backups.write`
permission. The result lists the purged backup IDs and `purged_bytes`.

Removing a server moves its backups along with it into **Removed and hidden**.
They are not touched by the trash expiry while the server is removed. See
[Removing and restoring servers](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace#removing-and-restoring-servers).

## Disk space

Backups compete with the servers for disk space. A few habits keep that safe:

- Watch the server's disk use on its Overview and the host's free space on
  [Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files).
- Set a keep count. Without one, every backup stays until you delete it.
- Keep the trash expiry on. Trashed backups use full disk space until they are
  purged.
- A restore needs room for the extracted server plus its safety backup.
- [Copying between servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers)
  refuses to start when the disk lacks room for the copy and the target's backup.

## Keep a copy somewhere else

Helix backups protect against a bad update, a broken plugin, or a deleted world.
They do not protect against losing the disk, because they usually sit on the same
one. For anything you cannot replace, copy archives to another machine or
removable storage on a schedule, for example with the download API above, `rsync`
over SSH, or your existing backup tool pointed at `backup_root`. Copy the
matching `.json` file too.

## Measured example

> [!NOTE]
> These numbers come from one test server on one machine. They show the kind of
> difference 1.10.0 made; your times and sizes will differ.

On a 16.7 GB Minecraft test server:

| | Before 1.10.0 | 1.10.0 |
| --- | --- | --- |
| Time | about 20 minutes, single-threaded | about 5 minutes |
| Archive size | 9.5 GB | 3.2 GB |

Most of the size drop came from leaving out a 6.45 GB Files trash. On fast NVMe
storage, a live backup of a running server finished in about 40 seconds with no
restart.

## Related pages

- [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes): deploys take a backup first.
- [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace): software updates also back up first.
- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery): backing up Helix's own state.
