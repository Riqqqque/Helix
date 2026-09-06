# Server software versions and repairs

Open a native Minecraft server, then **Advanced → Server software**.

- **Repair current runtime** downloads the exact installed artifact again and
  rebuilds Forge/NeoForge loader libraries. It checks the saved SHA-256 before
  replacing anything. This repairs runtime files, not broken mods or worlds.
- **Choose version / update build** lists published versions for the server's
  current software. Choose the current Minecraft version for its newest supported
  build, or a newer Minecraft version. Pumpkin lists its published releases.
- **Backups & restore** opens the full backups for this server.

Confirm the change, then click **Back up & repair** or **Back up & change version**.
Downloads and loader installation are staged before the server stops. A full,
verified backup must finish before activation. Helix changes the runtime and its
Java image together; worlds, settings, plugins and mods are kept.

A running server restarts and must pass its startup check. If activation or that
check fails, Helix restores the complete backup, including the world, rather than
only swapping back a JAR. Failed data remains available for investigation.
A stopped server stays stopped: startup is not tested until it is started. If it
fails then, restore the safety backup from **Backups**.

Refreshing the browser does not cancel the background job. Follow it in
**Background activity**. A host or broker interruption is not the same as a page
refresh: inspect the interrupted job and safety backup before retrying.

## Compatibility limits

Check plugin and mod support before upgrading Minecraft. Helix cannot guarantee
that third-party code supports a newer version. Existing worlds are not safely
downgradable, so backward and unordered snapshot migrations are rejected. Restore
a matching full backup or test a copy in a separate server instead.

This does not switch software families. Modpack Minecraft and loader versions stay
pinned: use the pack's update controls to upgrade them together. Exact runtime
repair is still available. Custom JARs have no trusted publisher download; back up,
stop the server, and replace the JAR through **Files**. Other games keep their own
update controls.

The API is `POST /api/v1/servers/{instance_id}/runtime`, requiring `games.manage`,
a valid session and CSRF protection. Its body contains `version` (null for exact
repair), `expected_version`, `expected_build`, and `confirmation_name`. Stale
requests are rejected and changes share the server's exclusive operation lock.
