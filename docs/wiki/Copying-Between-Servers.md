# Copying Between Servers

Build and test on one server, then move the finished setup to another. A common
use is a private test server and a public production server: try new plugins and
settings on the test server, then copy them over in one step.

The source server is only read. The target gets a full backup first, and if
anything goes wrong — including the target not starting afterwards — Helix
restores that backup automatically.

> [!NOTE]
> This feature is new in Helix 1.7. The check step and the safe file packing are
> tested against real servers; a complete copy between two live servers has not
> been run end to end yet. Keep an off-host backup of anything irreplaceable.

## From the dashboard

1. Open the server you want to copy **from**, then **Advanced → Copy to another
   server**.
2. Pick the server to copy **to**.
3. Tick what to copy.
4. Choose **Check what will be copied**. Helix lists the files and sizes for each
   part, the plugin and mod JARs involved, and anything that would stop the copy.
5. Type the target server's name and choose **Copy to …**.

The target stops while files are copied and starts again if it was running, so
players are disconnected for a minute or two. The result lists how many files
were copied, which older JARs were replaced, and where the backup is.

## What you can copy

| Part | Copies | Default |
| --- | --- | --- |
| Plugins and their settings | Plugin JARs and their config files | On for plugin servers |
| Mods | Every JAR in `mods/` | On for mod servers |
| Server configuration | `bukkit.yml`, `spigot.yml`, Paper, Purpur, Pufferfish, and Leaves files, `config/`, `defaultconfigs/` | On |
| Datapacks | The world's `datapacks/` folder | On |
| Gameplay settings | `server.properties` values such as difficulty, PvP, and view distance | Off |
| Whitelist, operators, and bans | `whitelist.json`, `ops.json`, and the ban lists | Off |
| All plugin data | Everything in `plugins/`, including databases and player files | Off |
| Worlds | The world, nether, and end folders | Off |

**Plugins and their settings** leaves plugin data behind on purpose: databases
(`*.db`, SQLite, H2), logs, caches, web map tiles, and player folders such as
`userdata`, `playerdata`, `players`, and `data`. That keeps the target's
CoreProtect history, LuckPerms data, and player balances intact. Choose
**All plugin data** only when you want the source's data to replace the target's.

**Worlds** replaces the target's world, nether, and end completely. The source
world is renamed to the target's world name if they differ.

## What the target keeps

- Its port, query port, RCON settings, and `server-ip`.
- Its world name, seed, player limit, MOTD, and whitelist switches, even when
  gameplay settings are copied.
- Every file you did not choose to copy. Nothing on the target is deleted
  unless it is replaced.

## Plugin and mod upgrades

Helix reads the name each plugin or mod declares inside its JAR
(`plugin.yml`, `paper-plugin.yml`, `fabric.mod.json`, `quilt.mod.json`, or
`mods.toml`). A new JAR replaces the target JAR with the same name, so
`MyPlugin-2.0.jar` replaces `MyPlugin-1.9.jar` instead of leaving two copies that
would stop the server from starting.

**Remove plugins or mods the source doesn't have** also deletes target JARs that
are missing from the source, so both servers end up with the same set.

## What Helix refuses

- Copying to an AMP or other imported server. Helix never writes AMP files;
  [bring the server into Helix](https://github.com/Riqqqque/Helix/wiki/Server-Migration) first.
- Plugins onto a mod server, or mods onto a different loader.
- Server configuration between different kinds of server software.
- Worlds or datapacks onto an older Minecraft version, which would damage them.
- Pumpkin servers, which use their own plugin format.
- Copies larger than 64 GiB, or when the disk lacks room for the copy and the
  target's backup.

Different Minecraft versions on the two servers are allowed with a warning;
check that every plugin supports the target's version.

## How it stays safe

Helix packs the source's files inside a throwaway container that runs as the
source server's own user, with no network and a read-only view of the server
folder. Links inside the server folder cannot reach anything outside it. Only
regular files come out.

On the target, Helix takes and verifies a full backup, stops the server, and
writes every file without following links. It then restores ownership, starts
the server, and waits until it responds. Any failure restores the backup and the
server's previous running state.

## From the API

Scripts and AI assistants can do the same with a
[server API token](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens) that lists **both** servers. Full access
covers it; otherwise the token needs **Read files** plus **View** for the check,
or **Write files** for the copy.

Check first — this changes nothing:

```json
{"operation":"server_transfer_preflight","instance_id":"helix:SOURCE-UUID","target_id":"helix:TARGET-UUID","parts":["plugins","configs","datapacks"]}
```

Then copy. This returns a `job_id` to poll with `job_status`:

```json
{"operation":"transfer_server_content","instance_id":"helix:SOURCE-UUID","spec":{"target_id":"helix:TARGET-UUID","parts":["plugins","configs","datapacks"],"confirmation_name":"Production","remove_missing_jars":false}}
```

Part names are `plugins`, `plugin_data`, `mods`, `configs`,
`server_properties`, `datapacks`, `player_lists`, and `worlds`. The dashboard
routes are `POST /api/v1/servers/{id}/transfer/preflight` and
`POST /api/v1/servers/{id}/transfer`. Full field details are in
[`docs/SERVER-TOKENS.md`](https://github.com/Riqqqque/Helix/blob/main/docs/SERVER-TOKENS.md).
