# Valheim servers

Helix runs the Linux dedicated server in its own container. Each server has its own world data, password, resource limits, mods, backups and console history. AMP and other managers stay separate.

## Create a server

1. Open **Servers → New server → Valheim**.
2. Choose a name, memory and optional CPU limit. Start with 4 GiB of memory; larger mod lists may need more. Vanilla Valheim supports up to 10 players. Higher limits need a compatible mod and are not a built-in launch setting.
3. Choose a world name and join password. A blank password during creation generates one; it is available in the server's Settings afterward.
4. Leave ports on Automatic, or choose two consecutive UDP ports from your Valheim pool.
5. Choose Steam-only or Crossplay, then create the server. SteamCMD installs the current stable build on the first boot. Later starts reuse it; game updates are manual.

The first install includes downloading SteamCMD and the server, then generating the world. It is not instantaneous. Helix waits for the game's connection message instead of declaring success after an arbitrary delay. A SteamCMD cold-start metadata failure gets a bounded retry.

## Joining

**Steam-only:** use the host's LAN IP and game port on your own network. For players outside the LAN, manually forward the game UDP port and the next UDP port to that host. The defaults are **2456–2457 UDP**. Helix can prepare its host firewall rules but never changes the router.

**Crossplay:** use the code in Console, the in-game list, or the public-IP method supported by the game. PlayFab relays do not require router forwarding. Local and loopback IP connections do not work in crossplay mode. Crossplay is separate from showing the server in the public list. Xbox players cannot install client-side mods.

## Settings and worlds

Stop the server before saving launch settings. Saving does not restart it. **Save for next start** writes `valheim.json`; the next start uses the saved values. Helix rejects stale saves if another editor changed the file, and keeps the previous configuration alongside it.

Settings cover the world, password, server-list visibility, crossplay, save interval, world backup count and intervals, difficulty presets, individual modifiers and world-rule keys.

A new world name creates or selects another world; it does not rename or remove an existing one. Leaving the preset blank keeps the world's saved rules. A selected preset resets its modifiers on every boot before applying your overrides. Unchecking a world-rule key only stops forcing it on: choose a preset to clear previously saved rules. Back up before experimenting with world rules.

### Importing a world

Always stop and back up before replacing saves.

- **Valheim 1.0:** copy the entire named folder under `worlds_local`, including its chunk files, `.db2`, `.fwl2` and checkpoint files. Do not select only the largest file. In Files, create the matching world folder and upload all its files, or transfer the complete folder through your existing server access.
- **Older saves:** copy the matching `.db` and `.fwl` files together into `worlds_local`. Let Valheim perform its own format conversion; retain the original backup.
- Set World name to the imported world's name before starting. To choose a specific seed, create that world in the game first and import it; Helix does not invent an unsupported seed launch flag.

Full Helix backups preserve the complete world directory, including the new chunked format. Restore through **Backups**, not by mixing files from different saves. Valheim's world-only backups are separate from Helix's full-server backups.

### Admins, bans and the allowlist

Settings has a shortcut to the access files in the server root:

- `adminlist.txt`: administrators.
- `bannedlist.txt`: blocked players.
- `permittedlist.txt`: allowed players. A non-empty list excludes everyone else.

Use one case-sensitive `Platform_UserID` per line, as shown in a player's F2 screen or server log. Vanilla Valheim does not provide an ordinary stdin/RCON command console. Helix's Console shows persistent output; use the game's supported administrator commands in game.

## Mods and modpacks

Open **Mods**, paste a Valheim Thunderstore package link or `Author-Package-Version`, and select **Preview**. Read the package's server/client requirements before installing.

**Back up & install** installs the chosen release, resolves declared dependencies and supplies the Valheim BepInEx runtime. Dependency-only Thunderstore modpacks are supported. Packages must belong to Thunderstore's Valheim community. The installer accepts package files, not arbitrary remote shell scripts.

- Stop first. A full, integrity-checked backup is required before an install, update, enable/disable or removal.
- Downloads and extraction are bounded. Path traversal, symlinks, duplicate archive paths and mismatched package manifests are rejected.
- Files are assembled in a new mod generation. The active inventory changes atomically only after preparation succeeds. A failed preparation leaves the previous generation selected.
- Existing configuration in `BepInEx/config` is preserved during updates. Open **Edit mod configs** to change it.
- Use **Check mod updates**, then choose which updates to install. Nothing silently updates your mods.
- Disabling or removing a required dependency is blocked with the dependent package's name. Dependencies are not silently uninstalled with their parent; review them separately.
- Removing a mod keeps its configuration and world data. Removing a content mod can still make a world unsafe to play; restore a matching full backup when needed.
- Local DLLs and their asset folders belong in `plugins`, after BepInEx is installed. Helix cannot check dependency versions or updates for manually uploaded files.

Enabled means selected for the next boot, not proven compatible. Check Console for loader errors. Some mods need matching client installs, some work only on clients, and game updates can break mods. Helix cannot guarantee compatibility for arbitrary third-party code.

## Updates and repair

In Settings, use **Back up & update** for the current stable Steam build or **Back up & repair** to also validate game files. Both require a stopped server and leave it stopped afterward. Start it when ready and inspect Console. If the new build or a mod fails, restore the pre-change full backup from Backups.

The Steam public branch does not expose an unrestricted version picker. An older full backup is the reliable way to return to the exact server files you previously ran. Never load a newer world with an older game build without a matching world backup.

Older Helix Valheim runtime containers must be upgraded from Advanced while stopped before using the new management controls. Upgrading the container does not silently install arbitrary mods.

## API

`POST /api/v1/servers/{instance_id}/valheim` accepts:

| Action | Purpose |
| --- | --- |
| `status` | Saved settings, revision, installed packages and running state |
| `save_settings` | Save validated settings with `expected_revision` |
| `package` | Preview a package using `reference` |
| `install` | Install a package/release and its dependencies |
| `set_mod_enabled` | Set `package` and `enabled` |
| `remove_mod` | Remove `package`, retaining configuration |
| `check_updates` | Background update check; no files changed |
| `update_game` | Update stable game files; set `repair: true` to validate |

Install, toggle, remove, update checks and game updates return `job_id`. Poll the existing jobs API; an accepted request is not a completed operation. Jobs keep running if the browser disconnects.

Owner requests require the usual session, CSRF and Origin checks plus `games.manage`. Scoped server tokens can use `valheim_manage` through the automation endpoint: settings actions need `settings`; package and software actions need `update`. Tokens remain restricted to their selected instance IDs. The existing server files, logs, start/stop/kill, resource controls and backup APIs also apply to Valheim.

See [SERVER-API.md](SERVER-API.md), [SERVER-TOKENS.md](SERVER-TOKENS.md) and [openapi.json](openapi.json).

## References

- [Iron Gate's dedicated-server guide](https://www.valheimgame.com/support/a-guide-to-dedicated-servers/): launch flags, ports, crossplay and access files.
- [Valheim 1.0 FAQ](https://www.valheimgame.com/support/valheim-1-0-faq/): supported player count and release behavior.
- [Valheim BepInEx package](https://thunderstore.io/c/valheim/p/denikson/BepInExPack_Valheim/): the game-specific loader and Linux launch requirements.
- [Thunderstore package format](https://wiki.thunderstore.io/mods/creating-a-package): manifests and dependencies.
