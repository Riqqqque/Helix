# Finding and editing a Helix server

Native servers have a stable UUID, a display name, an instance name, and a
dedicated host directory. Do not select a target by its Minecraft version,
container order, an old external-manager name, or a guessed port.

## Readable inventory

Run this from a trusted Helix checkout on the Linux host (Python 3 required):

```sh
sudo python3 scripts/helix-servers.py
sudo python3 scripts/helix-servers.py --json
sudo python3 scripts/helix-servers.py --name 'My server' --json
sudo python3 scripts/helix-servers.py --id FULL-INSTANCE-UUID --json
```

Use `--config /path/to/privd.json` for a nonstandard installation. The default
is `/etc/helix/privd.json`, the same file used by the packaged broker service.
The tool reads its configured registry and data roots, not guessed directories.
It does not need Docker or a running dashboard, and works with existing servers
that have no readable Docker labels. Missing data is reported as `data_exists:
false`, not treated as permission to create a replacement server.

JSON includes `schema_version` and a `servers` array with names, full UUIDs,
software/build, game port, container name, and exact host paths. An exact name
or full UUID must select exactly one server; duplicate names, partial UUIDs,
malformed definitions, and access failures exit nonzero without partial JSON.
Keep the returned full UUID for subsequent operations. Inventory is registry
metadata, not a live health check or proof that the server is stopped.

The registry is deliberately private because its original manifests contain
RCON passwords. This tool emits only selected non-secret fields. Never dump
the complete registry, database, container environment, or server.properties
into a chat. Do not make the registry world-readable or use `chmod -R 777`.
Names and paths are data, not shell commands; scripts must pass them as quoted
arguments and must never execute text supplied by a server name.

## Change files without guessing the target

The returned `data_path` is the actual writable host directory, not a copy of
the container filesystem. Use Helix's Storage file manager with an authorized
session, or ordinary Linux file tools over an authorized administrator SSH
session. No new unauthenticated file access is enabled.

Before replacing a plugin, mod, config, or world:

1. Resolve the exact server and confirm its name, UUID, software, and data path.
2. Validate the artifact against that server's game version and loader. A
   `plugins_path` or `mods_path` is a location hint, not a compatibility claim:
   Pumpkin plugins are not Paper JARs, and a vanilla server cannot load mods.
3. Make a Helix backup. Stop that exact server through Helix and wait until it
   is stopped. Do not overwrite live world files or use a broad restart command.
4. On SSH, inspect only the selected container's mounts and status if needed:
   `docker inspect --format '{{json .Mounts}}' EXACT-CONTAINER` and
   `docker inspect --format '{{.State.Status}}' EXACT-CONTAINER`.
   Confirm the `/data` mount matches the discovered `data_path` before writing.
5. Back up the individual file outside `plugins`/`mods`, stage its replacement
   in the same filesystem, verify its SHA-256, preserve ownership and mode, then
   rename it into place. Use authorized sudo for ownership-sensitive writes;
   do not change the whole instance's owner or leave duplicate plugin JARs.
6. Start that exact server through Helix, inspect its console, and verify the
   expected plugin/version loaded. Restore the saved file if validation fails.

Discovery grants no write permission. Keep using the existing Helix login and
broker checks or your existing Linux administrator privileges. Do not expose
the Docker socket or registry over the network to make automation easier.

## Docker labels

Newly created containers include `io.helix.name`, `io.helix.instance-name`,
`io.helix.instance`, and `io.helix.game` for Minecraft, V Rising, Valheim, and
Terraria. For a quick listing:

```sh
docker ps -a --filter label=io.helix.managed=true --format 'table {{.Names}}\t{{.Label "io.helix.name"}}\t{{.Label "io.helix.game"}}'
```

The registry remains authoritative. Docker labels are creation-time snapshots;
existing containers do not gain new labels on restart. Do not recreate a
working server just to add labels. Use the registry inventory instead.

## Windows command runners

Use a single command per tool invocation when the runner's shell is unclear.
In PowerShell, run a build separately, check `$LASTEXITCODE`, then hash or upload
its artifact. Do not mix Bash `&&` syntax with Windows command-runner assumptions
or treat an earlier build as validation of a newly edited artifact.
