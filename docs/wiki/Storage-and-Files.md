# Storage and Files

Storage shows the configured disks and mount points reported by the Linux host.
The file manager can operate only inside roots explicitly allowed by the
root-owned broker configuration; it is not an arbitrary root filesystem
browser.

## Browsing

Open a drive or folder to see separate Name, Size, Type, Modified, and Actions
columns. Choose 25, 50, 100, or 200 rows per page. Large directory reads have a
bounded loading state and retry path; navigating successfully does not also show
a stale timeout error.

Regular file sizes come from metadata. A folder's full recursive size is not
calculated during ordinary browsing because doing that for every row can make a
large disk unusably slow. Use **Analyze this folder** when recursive totals are
needed.

The action menu separates:

- **Open/Edit text** for validated UTF-8 regular files up to 4 MiB;
- **Rename** for files or folders;
- **Move to trash** for recoverable deletion; and
- ordinary folder navigation.

Binary media such as MP4 files never opens in the text editor. The UI explains
the unsupported type instead of treating Rename and Edit as the same action.

## Size analysis

Every mounted physical-drive card has an **Analyze space** action. It opens the
analyzer at that drive's primary mount and selects Thorough mode without
starting any work until the user confirms. **Analyze current folder** is also
available above Files for a narrower, faster scan.

Quick scan is capped at 30 seconds, 250,000 entries, and depth 64. Thorough scan
is opt-in and capped at 10 minutes, 5,000,000 entries, depth 128, and one active
job. Both read metadata only, avoid symbolic links, and stay on the selected
filesystem. Closing the analyzer does not cancel the job; reopening the same
path in that browser resumes its latest retained job.

The privileged broker keeps read-only `analysis_roots` separate from writable
`managed_roots`. A typical installation uses `/` as its analysis root so every
mounted drive can be inspected, while create, edit, rename, and trash operations
remain limited to explicitly managed storage directories.

The result distinguishes two ideas:

- **scan coverage** says whether every eligible entry was considered; and
- **ranking retention** says how many top rows fit in the bounded response.

A full-coverage result can omit millions of small ranking rows while still
knowing which retained files are the largest. A partial-coverage result explains
whether time, entry depth, permissions, concurrent changes, or cancellation
prevented a complete answer. Run Thorough when Quick reaches a safety limit.

Results include largest files, largest recursive folder trees, and largest
direct folder contents. The default ranking uses allocated filesystem blocks,
which reflects consumed space more accurately than logical length for sparse
files. Both allocated and logical sizes remain visible. Hard-linked aliases are
visited for coverage but their shared inode is charged only once, so Docker and
backup trees do not inflate disk usage. Filesystem metadata or reserved space is
not attributed to an individual file. Results are sortable and paginated. A file row
can open its containing folder or move that exact file to recoverable trash
after a red confirmation.

## Recovery and performance

Trash operations move data into a protected trash root; they do not claim an
irreversible erase. Keep independent backups anyway. A multi-terabyte analysis
can still create real disk I/O, especially on a hard drive, so start it when that
load is acceptable and cancel it from the progress view if needed.
