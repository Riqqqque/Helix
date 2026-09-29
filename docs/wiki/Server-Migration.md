# Copy a server into Helix

Helix never takes over an AMP or Pterodactyl instance. **Copy an existing
server** creates a new native Helix server, copies the world, plugins, mods, or
saves into it, and starts it. The old files stay where they are.

For Minecraft the copy is meant to feel like nothing moved:

- **Same version.** Helix reads the version the world last ran from the server's
  own files — its log, Paper's version history, or its version folders — instead
  of trusting the panel's settings, which often say "latest" or name a loader
  version. It refuses to install an older version, and never installs "latest"
  over an existing world.
- **Same address.** **Keep port** reuses the original game port when the stopped
  source is the only thing that claims it, so players and router forwarding keep
  working.
- **Same add-ons.** Ports opened by Simple Voice Chat, BlueMap, Dynmap, and
  Geyser are read from their configs and opened on the new server.
- **It starts.** A panel's `server-ip` setting, which binds to the host address
  and would stop the server inside its container, is cleared.

Stop the source first. A live copy can miss chunks or lock files.

## Easy path (AMP Minecraft)

1. Stop the instance in AMP, or use **Stop** on the imported connection in Helix.
2. Open that imported server and choose **Copy into Helix**, or choose
   **New server → Copy an existing server** and pick the AMP connection.
3. Choose **Inspect**. If Helix says it is still running, choose **Stop in AMP**
   and inspect again. Inspect only reads files.
4. Check the **Minecraft version** hint says *Same version the server last ran*,
   leave **Keep port** on, and look over the add-on ports Helix will open.
5. Confirm the Minecraft EULA, that the source is stopped, and that this is a
   copy into a new Helix server, then choose **Copy into Helix**.
6. Wait for the job. First Java copies are usually a few minutes. Steam games
   can take 10–30 minutes the first time. When it finishes, Helix shows the port
   players use and the add-on ports it opened, and lists any it could not.

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
| Minecraft | Worlds, plugins, mods, configs, datapacks, player lists. `server.properties` keeps its settings on Helix's ports and RCON, with `server-ip` cleared. Add-on ports are opened | AMP kvp, Java, logs, crash reports, backups, and loose launcher JARs such as `paperclip.jar`; Helix installs its own server JAR |
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
list, notes, and **blockers**. For Minecraft it also returns
`detected_version` (from the server's own files), `source_game_port`,
`source_port_available`, `source_port_problem`, `source_start_on_boot`, and
`plugin_ports` (each with `port`, `protocol`, `label`, `available`, and
`reason`). Do not start a copy while `running` is true or `blockers` is
non-empty.

Copy after the source is stopped:

```http
POST /api/v1/servers/migrate
Content-Type: application/json

{
  "source": {"kind":"amp","instance_id":"amp:11111111-1111-4111-8111-111111111111"},
  "name": "Survival Helix",
  "game": "minecraft",
  "software": "paper",
  "version": "1.21.8",
  "memory_mb": 4096,
  "game_port": 25565,
  "max_players": 20,
  "network_exposure": "private",
  "start_on_boot": true,
  "eula_accepted": true,
  "source_stopped": true,
  "copy_acknowledged": true
}
```

`source_stopped` and `copy_acknowledged` must be true. Helix still refuses if
AMP reports the instance online. Minecraft copies need the exact version the
world last ran, such as `1.21.8`; `latest` and anything older than
`detected_version` are refused. Set `game_port` to `source_game_port` to keep
the original address, or leave it out for a free port. Add-on ports are opened
automatically; the job result lists them under `extra_ports` and any that were
taken under `extra_ports_skipped`. The job starts the new server when the copy
and first boot finish. AMP and Pterodactyl are not deleted.

Stop an imported AMP instance first if needed:

```http
POST /api/v1/servers/amp:11111111-1111-4111-8111-111111111111/actions
Content-Type: application/json

{"action":"stop"}
```

## After the copy

If you kept the port, players join exactly as before. If Helix picked a new
port, update your router forwarding and tell players the new address.

Leave the old instance stopped. If AMP starts it with the host, turn off its
autostart, or both servers will try to use the same ports after a reboot. When
you are happy, retire the old instance in that manager; Helix will not do that
for you. After uninstalling AMP entirely, you can remove its folder from the
broker's `managed_roots`; Helix skips a missing folder with a warning either
way.

To move later changes from one Helix server to another — for example from a
test server to this one — use [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers).

Deep management (console, settings, marketplace, backups) exists only on the
new `helix:` server.
