# Storage and Files

The **Storage** page shows every physical drive on the host, lets you find
what is using space, and gives you a file manager. You can browse the whole
host, but Helix only changes files inside folders the broker configuration
allows. Deleting moves items to a recoverable trash folder; nothing is erased
on the spot.

## Storage overview

The top of the page has one card per physical disk:

- the drive model (or device name) and total size;
- its largest mount point and the free space there;
- a usage bar; and
- **Browse files** and **Analyze space** buttons.

A disk with no mounted filesystem shows **Not mounted** and no buttons.

Mount data comes from the host's own view of its filesystems and refreshes at
the dashboard's live-data interval. When a mount passes 90% full, a notice
appears in the bell menu at the top right and on Overview. Closing a notice
hides it in this browser only; **Settings → Helix data → Dismissed notices →
Show them again** brings it back.

## Two kinds of roots

The broker reads two lists from `/etc/helix/privd.json`:

| Key | Used for | Typical value |
| --- | --- | --- |
| `managed_roots` | Every change: create, edit, upload, rename, trash. Also the folders you can copy into a new server. | `/srv/helix/instances`, `/srv/storage`, `/srv/amp/instances` |
| `analysis_roots` | The read-only space analyzer. | `/` |

If `analysis_roots` is empty, the analyzer uses `managed_roots`. Up to 32 roots
can be configured. A root that does not exist on this host is skipped with a
warning.

Some paths are always hidden, even for browsing: `/proc`, `/sys`, `/dev`,
`/run`, `/etc/shadow`, `/etc/gshadow`, `/etc/security/opasswd`, and
`/root/.ssh`.

To make another folder writable, add it to `managed_roots`, add it to the
broker unit's `ReadWritePaths` with a systemd drop-in, then restart
`helix-privd`:

```bash
sudo systemctl edit helix-privd
# [Service]
# ReadWritePaths=/mnt/media
sudo systemctl restart helix-privd
```

Restarting the broker does not stop game containers.

## The file manager

### Browsing

- Use the breadcrumb or **Parent folder** to move around. **Refresh** reloads
  the current folder.
- Columns: Name, Type, Size, Modified, Mode, and actions.
- Choose 25, 50, 100, or 200 **Rows** per page, then use **Previous** and
  **Next**.
- **Filter this page** narrows the names on the current page.
- Folder sizes show as a dash. Helix does not add up every folder while you
  browse, because that would make a large disk crawl. Use **Analyze folder**
  for real totals.
- Entries that cannot be read or shown safely are counted in a note under the
  table.

### Actions

| Action | Where it works | Limits |
| --- | --- | --- |
| **New folder**, **New file** | Inside a managed root | Names up to 255 bytes. No `/`, `\`, control characters, `.` or `..`. |
| **Upload** or drag files onto the list | A writable folder | Regular files from 1 byte to 256 MiB. Folders are rejected. Sent in 2 MiB chunks, at most 2 uploads at once. An upload idle for 10 minutes is dropped. |
| Edit text | Inside a managed root | UTF-8 files up to 4 MiB with a known text extension (`.txt`, `.json`, `.yml`, `.properties`, `.toml`, `.conf`, `.sh`, and similar) or named `Dockerfile`, `Makefile`, `LICENSE`, or `README`. |
| **Rename** | Inside a managed root | Same name rules. Fails if the new name exists. |
| **Move to trash** | Inside a managed root | Asks for confirmation first. |

Buttons for items outside a managed root are disabled.

### Editing safely

- Binary files such as videos never open in the editor. The type column
  explains what they are.
- The editor shows the path, the size, and the 4 MiB limit as you type.
- Saving writes a temporary file next to the original, keeps its owner and
  permissions, then swaps it in. A failed save leaves the original untouched.
- If the file changed on disk after you opened it, the save is refused. Reload
  and try again.
- Closing with unsaved changes asks **Discard changes** or **Keep editing**.

### Symbolic links

Helix will not edit, rename, or trash a symbolic link. It also will not follow
one out of a managed root.

### Recoverable trash

**Move to trash** moves the item into a `.helix-trash` folder at the top of
its managed root. The new name is `<time>-<random id>-<original name>`, so
nothing collides. The result tells you the recovery path.

- To restore, move the item back from a shell (for example the Terminal
  page). The file manager can rename but not move items between folders.
- A managed root itself cannot be trashed.
- Helix does not empty `.helix-trash` for you. Clear it from the shell when you
  are sure.

This is a safety net, not a backup. Keep independent copies of anything you
cannot lose.

### Native server files

Each native server has its own **Files** tab on the Servers page. It uses the
same drop target, but paths are relative to that server and changes made while
the server runs are staged until you deploy them. See
[Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).

## The space analyzer

Open it from a drive card (**Analyze space**), from **Analyze current folder**
above the file list, or from **Analyze folder** in the file toolbar. Nothing
runs until you start the scan.

### Quick and Thorough

| | Quick | Thorough |
| --- | --- | --- |
| Time limit | 30 seconds | 10 minutes |
| Entries | 250,000 | 5,000,000 |
| Depth | 64 | 128 |
| Ranked rows per list | 128 | 2,048 |
| Running at once | 2 | 1 |

Drive cards open in Thorough mode; the folder buttons open in Quick mode. Both
modes:

- read metadata only, never file contents;
- do not follow symbolic links;
- stay on the filesystem you picked;
- count a hard-linked file once, so Docker and backup trees are not inflated;
  and
- rank by allocated disk blocks, with logical size shown beside it.

Closing the analyzer does not stop the scan. Reopening the same path in that
browser picks up the latest result. Use **Cancel scan** to stop one.

### Reading the result

The result has three ranked lists: largest files, largest folder trees, and
largest direct folder contents. Sort largest or smallest first, and page
through the rows.

Two ideas are kept apart:

- **Scan coverage** says whether every eligible entry was seen. A partial scan
  lists why: time, entry or depth limits, permission errors, files that changed
  during the scan, other filesystems, or cancellation.
- **Ranking rows** are just the top entries that fit in the response. A full
  scan can still return only the top 128 or 2,048 rows.

If Quick hits a limit, run Thorough.

From a file row you can **Show in files** or **Move to trash** (after a red
confirmation). Folder rows have **Open folder**. Trash only works inside a
managed root.

## Copying servers from other managers

Copying a Pterodactyl volume, or an AMP V Rising, Valheim, or Terraria folder,
into a new Helix server reads from `managed_roots`. Add
`/var/lib/pterodactyl/volumes` or `/home/amp/.ampdata/instances` when you want
those folders to be selectable. AMP Minecraft copies use the AMP connection
instead. See
[Server Migration](https://github.com/Riqqqque/Helix/wiki/Server-Migration).

## Performance

A Thorough scan of a multi-terabyte disk can cause noticeable disk activity,
especially on a hard drive. Start it when that is acceptable, and cancel it
from the progress view if needed.

## Related pages

- [Backups](https://github.com/Riqqqque/Helix/wiki/Backups)
- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery)
- [Troubleshooting and FAQ](https://github.com/Riqqqque/Helix/wiki/Troubleshooting-and-FAQ)
- [Glossary](https://github.com/Riqqqque/Helix/wiki/Glossary)
