# Copy a server into Helix

Helix never takes over an AMP or Pterodactyl instance. **Copy an existing
server** creates a new native Helix server and copies the world, plugins, mods,
or saves into it. The old files stay where they are. The new server gets a free
Helix port, so the old manager can keep its number until you retire it
yourself.

Stop the source first. A live copy can miss chunks or lock files.

## Easy path (AMP Minecraft)

1. Stop the instance in AMP, or use **Stop** on the imported connection in Helix.
2. Open that imported server and choose **Copy into Helix**, or choose
   **New server → Copy an existing server** and pick the AMP connection.
3. Inspect. If Helix says it is still running, stop it and inspect again.
4. Confirm the Minecraft EULA, that the source is stopped, and that this is a
   copy into a new Helix server.
5. Wait for the job. First Java copies are usually a few minutes. Steam games
   can take 10–30 minutes the first time.

Helix reads AMP’s Paper/Fabric/Forge type from the instance. Spigot/Bukkit
becomes Paper. Hybrid loaders (Mohist and friends) copy the existing JAR as a
custom server — type the exact Minecraft version, not latest. Bedrock stays in
AMP. If AMP is updating or still starting, Helix treats that as running and
refuses the copy.

## Folder path (Pterodactyl, AMP V Rising / Valheim / Terraria, manual)

1. Stop the server in Pterodactyl or AMP.
2. Make sure the parent folder is a helix-privd `managed_roots` path. The
   example config already has `/srv/amp/instances`. Pterodactyl volumes are
   usually `/var/lib/pterodactyl/volumes/<uuid>`. If Inspect says the folder is
   outside Storage, add that parent to `managed_roots` and reload helix-privd.
3. **New server → Copy an existing server → Folder on this host**, paste the
   absolute path, Inspect, then copy.

AMP V Rising, Valheim, and Terraria are not in Helix’s Minecraft-only AMP
inventory. Use the instance folder (`/home/amp/.ampdata/instances/Name` on a
typical AMP host). Helix looks inside for saves, worlds, and mods. Wine and
SteamCMD trees are skipped; Helix installs those games itself.

## What gets copied

| Game | Copied | Left behind |
| --- | --- | --- |
| Minecraft | Worlds, plugins, mods, configs. `server.properties` motd/world/whitelist merge onto Helix ports and RCON | AMP kvp, Java, logs, crash reports, backups, Helix then installs its own loader JAR |
| V Rising | Saves and host settings (`SaveName`, password, description kept). Helix rewrites ports, listing, and the Helix server name. AMP `save-data/Saves` lands in Helix `save/Saves`. | Wine prefix, SteamCMD, dedicated binaries |
| Valheim | Worlds (`worlds` and `worlds_local`) and BepInEx plugins. A copy named `Dedicated` is kept if the world used another name | SteamCMD, server binaries |
| Terraria | Worlds and `.tmod` files. A copy named `world.wld` is kept if the world used another name | SteamCMD, server binaries |

Limits: 128 GiB, 250,000 files, depth 24, no symbolic links.

## HTTP API

Needs a signed-in session, CSRF header, and `games.manage`. Public player
access also needs `network.firewall.write`. Poll `GET /api/v1/jobs/{job_id}`.

Inspect an imported AMP Minecraft instance:

```http
POST /api/v1/servers/migrate/preflight
Content-Type: application/json

{"kind":"amp","instance_id":"amp:11111111-1111-4111-8111-111111111111"}
```

Inspect a Pterodactyl volume or AMP V Rising / Valheim / Terraria instance
folder:

```http
POST /api/v1/servers/migrate/preflight
Content-Type: application/json

{"kind":"folder","path":"/var/lib/pterodactyl/volumes/11111111-1111-4111-8111-111111111111"}
```

AMP non-Minecraft instances use the instance folder, for example
`/home/amp/.ampdata/instances/VRising01`. That parent must be in helix-privd
`managed_roots`.

Preflight returns the detected game, software, file count, byte size, copy
list, notes, and **blockers**. Do not start a copy while `running` is true or
`blockers` is non-empty.

Copy after the source is stopped:

```http
POST /api/v1/servers/migrate
Content-Type: application/json

{
  "source": {"kind":"amp","instance_id":"amp:11111111-1111-4111-8111-111111111111"},
  "name": "Survival Helix",
  "game": "minecraft",
  "software": "paper",
  "version": "latest",
  "memory_mb": 4096,
  "max_players": 20,
  "network_exposure": "private",
  "start_on_boot": true,
  "eula_accepted": true,
  "source_stopped": true,
  "copy_acknowledged": true
}
```

`source_stopped` and `copy_acknowledged` must be true. Helix still refuses if
AMP reports the instance online. Custom JAR copies need an exact Minecraft
version such as `1.21.8`. The job starts the new server when the copy and first
boot finish. AMP and Pterodactyl are not deleted.

Stop an imported AMP instance first if needed:

```http
POST /api/v1/servers/amp:11111111-1111-4111-8111-111111111111/actions
Content-Type: application/json

{"action":"stop"}
```

## After the copy

Join the new Helix port, not the old AMP/Pterodactyl port. When you are happy,
stop and retire the old instance in that manager. Helix will not do that for
you.

Deep management (console, settings, marketplace, backups) exists only on the
new `helix:` server.
