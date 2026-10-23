# Dashboard API Reference

This page lists every route of the owner API: the routes the dashboard uses,
which a signed-in owner session can also call from a script. Each entry gives
the method, path, the capability Helix checks, the body or query fields, and
what the route does.

For how sign-in, errors, limits and jobs work, read
[API and Integrations](https://github.com/Riqqqque/Helix/wiki/API-and-Integrations)
first. Token automation uses a different endpoint, described in
[Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API).

> [!CAUTION]
> Helix is a private-LAN release. These routes control your host. Keep the
> dashboard on a network you control and do not expose it to the internet.

## Reading this page

- Paths are relative to your dashboard address, for example
  `http://192.0.2.10:3100`. All API paths start with `/api/v1`.
- **Capability** is what the signed-in account must hold. The owner holds all
  of them. "Session" means any signed-in session.
- Unless a row says otherwise, every route needs the `helix_session` cookie and
  the `X-Helix-CSRF` header, and every `POST`, `PUT` and `DELETE` also needs
  `Content-Type: application/json` and the dashboard `Origin`.
- "No body" means the route ignores the body. You still send the content type.
- "Job" means the route returns `{"job_id": "...", "reused": false}`. Follow it
  with the status route named in that section.
- Bodies are strict: unknown fields are refused with `400 invalid_json`.
- When a request is refused by the host broker, the response is
  `400 broker_operation_rejected` with a message naming the cause.

## Authentication and capabilities

| Capability | Covers |
| --- | --- |
| `system.view` | Overview, host inventory, health, Docker inventory, hooks, security center, Strands list and calls |
| `system.settings.write` | Host settings, Docker container actions and cleanup, hook installs and service actions, security switches, Strand installs, server API tokens |
| `system.power` | Host reboots and reboot schedules |
| `system.packages.read` | Package inventory, package jobs, Helix update check |
| `system.packages.write` | Package refresh and apply, Helix update apply |
| `storage.files.read` | Browse and read files in the configured storage roots |
| `storage.files.manage` | Create, write, rename, trash and upload files in storage roots |
| `storage.analyze` | Folder size analysis |
| `network.firewall.read` | Network inventory |
| `network.firewall.write` | Firewall rules and activation; public game-server exposure |
| `games.view` | Server inventory, detail, logs, settings, marketplace search, backups list, jobs |
| `games.manage` | Server creation, lifecycle, console, files, deploy, settings, ports, marketplace installs, transfers, removal |
| `games.backups.manage` | Backup restore, trash, keep rules, backup downloads |
| `dashboard.customize` | Preferences, weather, Homarr import |
| `terminal.open` | The host terminal |
| `machines.view` | Rack machine list and probes |
| `machines.manage` | Add, change, remove, wake, power and open a terminal on rack machines |
| `users.manage` | Account changes and session expiry setting |

A few routes use other checks:

- **No authentication:** `GET /healthz`, `GET /api/v1/setup/status`,
  `POST /api/v1/setup/owner`, `POST /api/v1/auth/login`. The two `POST` routes
  still need the JSON content type and `Origin`.
- **Session cookie, no CSRF header:** the marketplace image proxies, Strand UI
  files, and the two terminal WebSocket routes. Browsers load these directly.
- **Password recheck:** `POST /api/v1/terminal/ticket`,
  `POST /api/v1/machines/{machine_id}/terminal/ticket`,
  `POST /api/v1/auth/server-tokens/{id}/reveal`, and `POST /api/v1/auth/account`
  take your current password in the body.

## Body limits

| Routes | Limit |
| --- | --- |
| Host, network, packages, hooks, Docker, security, marketplace, storage files, servers, jobs | 5 MiB |
| `POST /api/v1/network/firewall/rules` | 8 KiB |
| `POST /api/v1/storage/analysis` | 8 KiB |
| `POST /api/v1/system/packages/apply` | 384 KiB |
| `POST /api/v1/servers/minecraft/modpacks` | 16 KiB |
| `/api/v1/servers/{instance_id}/appearance` | 768 KiB |
| Setup, auth, account, terminal | 16 KiB |
| `/api/v1/settings/preferences` | 80 KiB |
| Strands | 12 MiB |
| Rack machines | 2 MiB (framework default) |
| Server tokens and automation | 5 MiB |

The bundled gateway also caps every request body at 12 MiB (6 MiB before 1.10.2, which blocked large Strand packages).

## Health, setup and session

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/healthz` | None | | Bodyless `204` liveness check. No database or host detail |
| `GET` | `/api/v1/setup/status` | None | | `ownerExists`, `bootstrapAvailable`, `bootstrapExpiresAtUnixMs` |
| `POST` | `/api/v1/setup/owner` | None (one-time bootstrap token) | `bootstrapToken`, `loginName`, `displayName`, `password` | Creates the first owner. Works once |
| `POST` | `/api/v1/auth/login` | None | `loginName`, `password` | Sets the `helix_session` cookie. Returns `user`, `csrfToken`, `expiresAtUnixMs`, `sessionExpires`. Rate limited per client and per account |
| `GET` | `/api/v1/auth/me` | Session | | Current `user` (id, login name, display name, capabilities), `expiresAtUnixMs`, `sessionExpires` |
| `POST` | `/api/v1/auth/csrf` | Session | `{}` | Swaps the CSRF value; returns the new `csrfToken` |
| `GET` | `/api/v1/auth/session-expiry` | Session | | `expires`, `expiresAtUnixMs` |
| `PUT` | `/api/v1/auth/session-expiry` | `users.manage` | `expires` (boolean) | `false` keeps the session until logout or a password change, capped at 400 days. `true` returns to 30 minutes idle, 8 hours at most |
| `POST` | `/api/v1/auth/account` | `users.manage` + current password | `currentPassword`, `loginName`, `displayName`, optional `newPassword` | Changes login name, display name and optionally the password. Changing the password or account invalidates every server API token |
| `POST` | `/api/v1/auth/logout` | Session | `{}` | Ends this session and clears the cookie |
| `GET` | `/api/v1/discovery` | Session | | API version, Helix version, session capabilities, authentication rules, conventions, links |
| `GET` | `/api/v1/openapi.json` | Session | | The OpenAPI document from `docs/openapi.json` |

## Overview, host and power

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/health` | `system.view` | | Dependency health |
| `GET` | `/api/v1/system/overview` | `system.view` | | CPU, memory, storage and network snapshot |
| `GET` | `/api/v1/host/inventory` | `system.view` | | Disks, mounts, interfaces, routes, services, processes, thread count, CPU model, listeners |
| `GET` | `/api/v1/host/integration` | `system.view` | | Docker and broker state, Helix container policies, reboot preflight and state |
| `PUT` | `/api/v1/host/integration/start-on-boot` | `system.settings.write` | `enabled` (boolean) | Sets the restart policy on the Helix dashboard and gateway containers only |
| `GET` | `/api/v1/host/reboot/preflight` | `system.view` | | Active players, running servers, jobs, and anything blocking a reboot |
| `POST` | `/api/v1/host/reboot` | `system.power` | `confirmation_hostname` (exact hostname), `delay_seconds` (0 for now, or 10–300), `disruption_acknowledged` (true) | Reboots now or schedules a reboot. Refused while players are online or jobs run |
| `DELETE` | `/api/v1/host/reboot/{operation_id}` | `system.power` | No body | Cancels a scheduled reboot while it can still be cancelled |
| `PUT` | `/api/v1/host/reboot/recurring` | `system.power` | `weekdays` (list of `monday`…`sunday`), `hour`, `minute`, `timezone`, `confirmation_hostname`, `disruption_acknowledged` | Creates or replaces the one recurring reboot schedule. Returns the host timezone and next run |
| `DELETE` | `/api/v1/host/reboot/recurring` | `system.power` | `confirmation_hostname` | Removes the recurring schedule |

See [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates).

## Preferences and weather

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/settings/preferences` | `dashboard.customize` | | Saved dashboard preferences and their revision |
| `PUT` | `/api/v1/settings/preferences` | `dashboard.customize` | `expected_revision` (integer), `preferences`: `navigation_order`, `metrics_refresh_ms`, `home_widgets`, optional `home_templates`, `active_home_id`, `colors` (`accent`, `text`, `surface`), `servers_enabled`, `hidden_pages` | Saves preferences. A stale revision is refused instead of overwriting another session's change. Rate limited |
| `GET` | `/api/v1/weather` | `dashboard.customize` | Query `location` (2–120 characters), `unit` (`celsius` or `fahrenheit`) | Current weather and forecast for one place |

Navigation sections are `overview`, `home`, `storage`, `network`, `host`,
`security`, `terminal`, `machines`, `servers`, `hooks`, `strands`, `globe`.
Home widget kinds are `clock`, `host`, `servers`, `storage`, `weather`, `note`,
`shortcut`, `graphs`, `docker`, `strand`, `globe`; each widget has `id`,
`kind`, `size` (`compact`, `wide`, `full`), `height` (`short`, `medium`,
`tall`), `title`, `content`, `url`, and optional `color`, `icon` and `layout`
(`x`, `y`, `w`, `h`). See
[Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home).

## Storage and files

These routes work only inside the storage roots set in the broker
configuration. They are not a general host filesystem API. For a game server's
own files, use the [server files route](#servers-files-and-deploy).

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/files` | `storage.files.read` | Query `path`, optional `cursor`, `limit` (default 50, up to 200) | Lists one folder |
| `POST` | `/api/v1/files/read` | `storage.files.read` | `path` | Reads a UTF-8 text file up to 4 MiB |
| `POST` | `/api/v1/files/directory` | `storage.files.manage` | `parent`, `name` | Creates one folder |
| `POST` | `/api/v1/files/file` | `storage.files.manage` | `parent`, `name` | Creates one empty file |
| `POST` | `/api/v1/files/write` | `storage.files.manage` | `path`, `content` (up to 4 MiB), optional `expected_modified_unix_ms` | Writes a text file; refused if it changed since that time |
| `POST` | `/api/v1/files/rename` | `storage.files.manage` | `path`, `new_name` | Renames one entry inside its storage root |
| `POST` | `/api/v1/files/trash` | `storage.files.manage` | `path` | Moves to the configured recoverable trash |
| `POST` | `/api/v1/files/upload/begin` | `storage.files.manage` for a folder; `games.manage` for a custom JAR | `target`: `{"kind":"directory","parent":"..."}` or `{"kind":"custom_jar"}`, `name`, `expected_size` | Starts a chunked upload. Folder uploads are 1 byte to 256 MiB; custom server JARs are 16 KiB to 768 MiB |
| `POST` | `/api/v1/files/upload/chunk` | Matches the upload's purpose | `upload_id`, `purpose` (`storage` or `custom_jar`), `offset`, `data_base64` (up to 2 MiB decoded) | Appends the next chunk in order |
| `POST` | `/api/v1/files/upload/finish` | Matches the upload's purpose | `upload_id`, `purpose` | Checks the size and publishes the file. Never overwrites |
| `POST` | `/api/v1/files/upload/abort` | Matches the upload's purpose | `upload_id`, `purpose` | Discards the unfinished upload |
| `POST` | `/api/v1/storage/analysis` | `storage.analyze` | `path`, `mode` (`quick` or `thorough`) | Starts a size analysis in the background |
| `GET` | `/api/v1/storage/analysis/{job_id}` | `storage.analyze` | | Progress and result |
| `DELETE` | `/api/v1/storage/analysis/{job_id}` | `storage.analyze` | No body | Asks the analysis to stop |

Two storage uploads can be active at once; an upload idle for 10 minutes is
discarded. Quick analysis stops at 30 seconds or 250,000 entries; thorough
analysis at 10 minutes or 5,000,000 entries, one at a time. See
[Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files).

## Network and firewall

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/network/inventory` | `network.firewall.read` | | Private addresses, router (UPnP) state, listeners, Docker publications, game ports, UFW state |
| `GET` | `/api/v1/network/globe` | `system.view` | | Country-level view of public connections. No remote addresses |
| `POST` | `/api/v1/network/firewall/rules` | `network.firewall.write` | `name`, `description`, `protocol` (`tcp` or `udp`), `port_start`, `port_end` | Adds a named UFW allow rule for one port or a range |
| `DELETE` | `/api/v1/network/firewall/rules/{rule_id}` | `network.firewall.write` | No body | Deletes a Helix-owned rule, keeping it briefly for undo |
| `POST` | `/api/v1/network/firewall/rules/{rule_id}/restore` | `network.firewall.write` | No body | Restores a deleted rule before the undo window ends |
| `POST` | `/api/v1/network/firewall/enable` | `network.firewall.write` | `ssh_port`, `confirmation` (`ENABLE UFW`) | Turns on UFW after keeping a listening SSH port open |
| `POST` | `/api/v1/network/amp-router-forwards/release` | `games.manage` + `network.firewall.write` | `port`, `confirmation` | Legacy. Always refuses and tells you to change the router yourself |

Helix never changes your router. See
[Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates).

## Packages and Helix updates

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/system/packages` | `system.packages.read` | | Installed and candidate versions, sizes, security and restart hints, cache age, Helix update readiness |
| `POST` | `/api/v1/system/packages/refresh` | `system.packages.write` | `{}` | Job: refreshes APT package lists |
| `POST` | `/api/v1/system/packages/apply` | `system.packages.write` | `packages` (list of `name`, `installed_version`, `candidate_version`), `confirmation`, `disruption_acknowledged` | Job: installs exactly the selected updates. Refused if it would remove or add packages. Never reboots |
| `GET` | `/api/v1/system/packages/jobs/{job_id}` | `system.packages.read` | | Refresh, apply and Helix update job progress and logs |
| `POST` | `/api/v1/system/helix/check` | `system.packages.read` | `{}` | Checks GitHub for a newer Helix release |
| `POST` | `/api/v1/system/helix/apply` | `system.packages.write` | `target_tag` (`vMAJOR.MINOR.PATCH`), `confirmation` (`UPDATE HELIX`), `disruption_acknowledged` | Job: updates Helix to that release |

## Docker

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/docker/inventory` | `system.view` | | Every container on the host with CPU and memory when reported, plus a Portainer hint |
| `POST` | `/api/v1/docker/actions` | `system.settings.write` | `name`, `action` (`start`, `stop`, `restart`), `confirmation` (the container name again) | Starts, stops or restarts one container. Helix's own containers are protected |
| `GET` | `/api/v1/docker/cleanup` | `system.view` | | Docker data root, disk use by category, policy, schedule, last run, active job |
| `POST` | `/api/v1/docker/cleanup/run` | `system.settings.write` | `retention_hours` (24–8760) | Job: removes old build cache, dangling images and unused networks. Never containers or volumes |
| `PUT` | `/api/v1/docker/cleanup/schedule` | `system.settings.write` | `weekdays`, `hour`, `minute`, `timezone`, `retention_hours` | Creates or replaces the cleanup schedule |
| `DELETE` | `/api/v1/docker/cleanup/schedule` | `system.settings.write` | No body | Removes the cleanup schedule |
| `GET` | `/api/v1/docker/cleanup/jobs/{job_id}` | `system.view` | | One cleanup job |
| `GET` | `/api/v1/docker/homarr` | `dashboard.customize` | | Imports Homarr shortcuts for the Home page |

## Security center

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/security` | `system.view` | | Host and Helix security facts and switches, with explanations |
| `POST` | `/api/v1/security/controls` | `system.settings.write` | `id`, `enabled`, `confirmation` (the control's phrase) | Flips one switchable control |

See [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery).

## Hooks

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/hooks` | `system.view` | | Configured services, the AMP adapter, and the Docker engine |
| `GET` | `/api/v1/hooks/{hook_id}/install/preflight` | `system.view` | | What an install would do, and anything blocking it |
| `POST` | `/api/v1/hooks/{hook_id}/install` | `system.settings.write` | `confirmation`, `repository_change_acknowledged` | Job: installs Tailscale or Jellyfin from its official signed repository |
| `GET` | `/api/v1/hooks/jobs/{job_id}` | `system.view` | | One install job |
| `POST` | `/api/v1/hooks/{hook_id}/actions` | `system.settings.write` | `action` (`start`, `stop`, `restart`, `enable`, `disable`) | Changes one allowlisted service and checks the result |

## Terminal

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/terminal/status` | `terminal.open` | | Whether the terminal service is ready |
| `POST` | `/api/v1/terminal/ticket` | `terminal.open` + current password | `currentPassword`, `columns` (20–400), `rows` (5–200) | Sets a 30-second, one-use ticket cookie |
| `GET` | `/api/v1/terminal/connect` | `terminal.open`; session cookie and ticket cookie, no CSRF | | WebSocket upgrade, subprotocol `helix-terminal-v1`, exact same `Origin` |

See [Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal).

## Rack machines

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/machines` | `machines.view` | | Registered machines |
| `POST` | `/api/v1/machines` | `machines.manage` | `label`, `host`, `port`, `username`, `authKind` (`key`, `password`, `system`), optional `notes`, `wolMac` | Registers a machine |
| `GET` | `/api/v1/machines/identity` | `machines.view` | | Helix's own SSH key for machines that use `key` |
| `POST` | `/api/v1/machines/refresh` | `machines.view` | No body | Probes every machine again |
| `PUT` | `/api/v1/machines/{machine_id}` | `machines.manage` | Same fields as create | Changes a machine |
| `DELETE` | `/api/v1/machines/{machine_id}` | `machines.manage` | No body | Removes a machine |
| `POST` | `/api/v1/machines/{machine_id}/probe` | `machines.view` | No body | Checks one machine over SSH |
| `POST` | `/api/v1/machines/{machine_id}/wake` | `machines.manage` | No body | Sends Wake-on-LAN to its `wolMac` |
| `POST` | `/api/v1/machines/{machine_id}/power` | `machines.manage` | `action` (`reboot` or `poweroff`), `confirmation` (the machine's label) | Reboots or powers off the machine over SSH |
| `POST` | `/api/v1/machines/{machine_id}/terminal/ticket` | `machines.manage` + current password | `currentPassword`, `columns`, `rows` | One-use ticket for an SSH terminal |
| `GET` | `/api/v1/machines/{machine_id}/terminal/connect` | `machines.manage`; session cookie and ticket cookie, no CSRF | | WebSocket upgrade for the SSH terminal |

## Strands

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/strands` | `system.view` | | Installed Strands |
| `POST` | `/api/v1/strands/inspect` | `system.settings.write` | `{"source":"upload","filename":"...","bytesBase64":"..."}` or `{"source":"url","url":"https://..."}` | Reviews a package without installing it |
| `POST` | `/api/v1/strands` | `system.settings.write` | Same as inspect | Installs or replaces a package. It starts disabled |
| `PUT` | `/api/v1/strands/{strand_id}` | `system.settings.write` | `enabled` (boolean) | Turns a Strand on or off |
| `DELETE` | `/api/v1/strands/{strand_id}` | `system.settings.write` | No body | Removes the package and its storage |
| `GET` | `/api/v1/strands/{strand_id}/package` | `system.view` | | Downloads the stored zip |
| `GET` | `/api/v1/strands/{strand_id}/files/{*asset}` | `system.view`; session cookie, no CSRF | | Serves a sandboxed UI file |
| `POST` | `/api/v1/strands/{strand_id}/host` | `system.view`, plus what the Strand declared | `method`, optional `params` | Runs a declared host call (`metrics.snapshot`, `storage.get`, `storage.set`, `storage.delete`, `storage.list`, `net.fetch`) for an enabled Strand |

See [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands).

## Server API tokens

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/auth/server-tokens` | `system.settings.write` | | Your tokens (`id`, `name`, `servers`, `permissions`, `created_at`, `expires_at`, `revoked_at`, `last_used_at`, `authorized`, `viewable`) and the list of `permissions` |
| `POST` | `/api/v1/auth/server-tokens` | `system.settings.write` + `games.manage` + `games.view`; `games.backups.manage` for backup permissions; `network.firewall.write` for `network`; both for `all` | `name` (1–80 characters), `servers` (1–64 exact IDs), `permissions`, `expires_in_days` (1–365, or null for never) | Issues a token. Returns `id`, `token`, `expires_at`, `viewable`. Invalid input is refused with `403 authorization_denied` |
| `DELETE` | `/api/v1/auth/server-tokens/{id}` | `system.settings.write` | No body | Revokes a token |
| `POST` | `/api/v1/auth/server-tokens/{id}/rotate` | Same as create for that token's permissions | No body | Issues a new value, keeping servers, permissions and expiry. Returns `token`, `viewable` |
| `POST` | `/api/v1/auth/server-tokens/{id}/reveal` | `system.settings.write` + current password | `current_password` | Returns the stored `token`. `404` if there is no viewable copy |

The token routes use snake_case field names.

The automation routes take a bearer token instead of a session. A request with
a session cookie or an `Origin` header is refused:

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `POST` | `/api/v1/automation/server` | Bearer token with the operation's permission | `operation`, `instance_id`, and the operation's fields | Runs one server operation. Two at a time |
| `GET` | `/api/v1/automation/jobs` | Bearer token | | The last 256 jobs started with this token |
| `GET` | `/api/v1/automation/jobs/{job_id}/wait` | Bearer token that started the job | Query `timeout_seconds` (0–12, default 12) | Returns when the job finishes, or after the timeout, with `finished` |

See [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API)
and [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens).

## Servers: inventory and readiness

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/servers` | `games.view` | | A JSON array of native and imported AMP servers. Use each `id` exactly |
| `GET` | `/api/v1/servers/inventory-health` | `games.view` | | Health of the imported AMP inventory |
| `GET` | `/api/v1/servers/manager/readiness` | `games.view` | | Native manager state, supported software and capabilities |
| `GET` | `/api/v1/games/readiness` | `games.view` | | Same as manager readiness |
| `GET` | `/api/v1/servers/{instance_id}` | `games.view` | | Native server detail, including `tps` when known and modpack provenance |
| `GET` | `/api/v1/servers/{instance_id}/capabilities` | `games.view` | | What this server and its manager support |
| `GET` | `/api/v1/jobs/{job_id}` | `games.view` | | One job record: `id`, `kind`, `status`, `stage`, `progress_percent`, timestamps, `result`, `error` |

## Servers: creation

All creation routes are jobs. They need `games.manage`, and also
`network.firewall.write` when `network_exposure` is `public`. `cpu_millis` is 0
(no cap) or 250–128000. Ports below 1024 are refused; leave a port out to take
the next free one from that game's pool.

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/servers/minecraft/versions` | `games.view` | Query `software` | Up to 128 published Minecraft versions for one software choice |
| `POST` | `/api/v1/servers/minecraft` | `games.manage` | `name`, `software`, `version`, `memory_mb` (1024–24576), optional `cpu_millis`, `max_players` (1–10000), optional `game_port`, optional `network_exposure` (`private` or `public`), `start_on_boot`, `eula_accepted` (true), optional `custom_jar` (`source_path`, `java_version`), optional `pumpkin_bedrock_port` | Creates a native Minecraft server |
| `POST` | `/api/v1/servers/vrising` | `games.manage` | `name`, `memory_mb` (2048–24576), optional `cpu_millis`, `max_players` (1–128), optional `game_port`, `query_port`, `network_exposure`, `list_on_browser` (default true), `start_on_boot` | Creates a V Rising server |
| `POST` | `/api/v1/servers/valheim` | `games.manage` | `name`, `memory_mb` (1024–16384), optional `cpu_millis`, `max_players` (10), optional `game_port`, `network_exposure`, `start_on_boot`, optional `settings` | Creates a Valheim server |
| `POST` | `/api/v1/servers/terraria` | `games.manage` | `name`, `software` (`vanilla` or `tmodloader`), `memory_mb` (512–8192), optional `cpu_millis`, `max_players` (1–255), optional `game_port`, `network_exposure`, `start_on_boot` | Creates a Terraria server |
| `POST` | `/api/v1/servers/palworld` | `games.manage` | `name`, `memory_mb` (4096–32768), optional `cpu_millis`, `max_players` (1–32), optional `game_port`, `query_port`, `network_exposure`, `list_on_browser`, `start_on_boot`, optional `server_password` | Creates a Palworld server |
| `POST` | `/api/v1/servers/native/{game}` | `games.manage` | `name`, `memory_mb`, optional `cpu_millis`, `max_players`, optional `game_port`, `query_port`, `network_exposure`, `list_on_browser`, `start_on_boot`, and the game's own optional fields: `server_password`, `admin_password`, `cluster_token`, `caves`, `world_seed`, `world_size`, `world_name` | Creates a server for `satisfactory`, `project_zomboid`, `seven_days_to_die`, `rust`, `sons_of_the_forest`, `factorio`, `dont_starve_together`, `vintage_story` or `hytale`. Fields a game does not use are refused |
| `GET` | `/api/v1/servers/minecraft/modpacks/search` | `games.view` | Query `query`, `offset`, `limit` (default 20, 1–50), `provider` (`modrinth` or `curseforge`) | Modpack search |
| `GET` | `/api/v1/servers/minecraft/modpacks/projects/{project_id}` | `games.view` | Query `provider` | Modpack versions and compatibility |
| `POST` | `/api/v1/servers/minecraft/modpacks` | `games.manage` | `name`, `memory_mb`, optional `cpu_millis`, `max_players`, optional `game_port`, `network_exposure`, `start_on_boot`, `eula_accepted`, `project_id`, `version_id`, optional `provider` | Creates a server from a modpack's server files |

Memory and player bounds for the shared games: Satisfactory 6–32 GiB, 1–16
players; Project Zomboid 4–32 GiB, 1–128; 7 Days to Die 6–48 GiB, 1–64; Rust
8–64 GiB, 1–500; Sons of the Forest 6–32 GiB, 1–8; Factorio 1–16 GiB, 1–255;
Don't Starve Together 1–8 GiB, 1–64; Vintage Story 2–32 GiB, 1–64; Hytale
4–32 GiB, 1–100. See
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace)
and [Game Hosting and Capacity](https://github.com/Riqqqque/Helix/wiki/Game-Hosting-and-Capacity).

## Servers: port pools

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/servers/port-policies/minecraft` | `games.view` | | Minecraft TCP ranges, priority ports, assignments and next free port |
| `PUT` | `/api/v1/servers/port-policies/minecraft` | `games.manage` | `game`, `ranges` (list of `start`, `end`), `ports`, `auto_forward_on_create` | Saves the Minecraft pool |
| `GET` | `/api/v1/servers/port-policies/vrising` | `games.view` | | V Rising UDP pool (game and query pairs) |
| `PUT` | `/api/v1/servers/port-policies/vrising` | `games.manage` | Same as Minecraft | Saves the V Rising pool |
| `GET` | `/api/v1/servers/port-policies/valheim` | `games.view` | | Valheim UDP pool (game port and the next two) |
| `PUT` | `/api/v1/servers/port-policies/valheim` | `games.manage` | Same as Minecraft | Saves the Valheim pool |
| `GET` | `/api/v1/servers/port-policies/terraria` | `games.view` | | Terraria TCP pool |
| `PUT` | `/api/v1/servers/port-policies/terraria` | `games.manage` | Same as Minecraft | Saves the Terraria pool |
| `GET` | `/api/v1/servers/port-policies/palworld` | `games.view` | | Palworld pool |
| `PUT` | `/api/v1/servers/port-policies/palworld` | `games.manage` | Same as Minecraft | Saves the Palworld pool |
| `GET` | `/api/v1/servers/port-policies/{game}` | `games.view` | | Pool for one of the shared games |
| `PUT` | `/api/v1/servers/port-policies/{game}` | `games.manage` | Same as Minecraft | Saves that pool |

`auto_forward_on_create` now only asks Helix to prepare host firewall rules for
new servers. It never changes the router.

## Servers: lifecycle, console and logs

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `POST` | `/api/v1/servers/{instance_id}/actions` | `games.manage` | `action` (`start`, `stop`, `restart`, `kill`, `update`, `backup`) | Job for native servers. Imported AMP servers go through the AMP adapter |
| `GET` | `/api/v1/servers/{instance_id}/logs` | `games.view` | Query `lines` (default 300, 1–1000) | Recent console lines |
| `GET` | `/api/v1/servers/{instance_id}/logs/history` | `games.view` | Query `cursor`, `lines` (default 200, 1–500) | Paged retained console history, newest page first |
| `POST` | `/api/v1/servers/{instance_id}/console` | `games.manage` | `command` (one line, under 512 bytes) | Runs a console command on a running Minecraft or Hytale server |

The [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API#lifecycle)
explains what each action does and returns.

## Servers: files and deploy

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `POST` | `/api/v1/servers/{instance_id}/files` | `games.view` + `games.manage` (every action, including reads) | A file command: `action` plus its fields | Server-relative file access. Two requests at a time |
| `POST` | `/api/v1/servers/{instance_id}/deploy` | `games.manage` | `backup`, `start`, `expect_log`, `revert_on_failure`, all optional. Send `{}` for the defaults | Job: installs staged file changes with backup, health check and rollback |

The file actions are `list`, `stat`, `read`, `download`, `create`, `mkdir`,
`write`, `move`, `trash`, `upload_file`, `upload_begin`, `upload_chunk`,
`upload_status`, `upload_finish`, `upload_abort`, `pending_list` and
`pending_discard`. They take the same fields as the token operation
`server_files`, documented in full in
[Automation API: Server files](https://github.com/Riqqqque/Helix/wiki/Automation-API#server-files).
Deploy options and results are in
[Automation API: Deploy](https://github.com/Riqqqque/Helix/wiki/Automation-API#deploy).

## Servers: settings, resources and network

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/servers/{instance_id}/settings` | `games.view` | | Minecraft settings, revision, field metadata and restart state |
| `POST` | `/api/v1/servers/{instance_id}/settings` | `games.manage` | `expected_revision`, `motd`, `game_mode`, `difficulty`, `max_players`, `view_distance`, `simulation_distance`, `player_idle_timeout`, `online_mode`, `pvp`, `allow_flight`, `white_list`, `enforce_white_list`, `spawn_protection`, `game_port`, `memory_mb` | Saves Minecraft settings. A `game_port` change rewrites the port settings and recreates the container |
| `POST` | `/api/v1/servers/{instance_id}/valheim` | `games.manage` | `action` and its fields: `status`, `save_settings`, `package`, `install`, `set_mod_enabled`, `remove_mod`, `check_updates`, `update_game` | Valheim settings and mods. Mod and update actions are jobs. See [Valheim](https://github.com/Riqqqque/Helix/wiki/Valheim) |
| `PUT` | `/api/v1/servers/{instance_id}/memory` | `games.manage` | `memory_mb` | Changes memory and recreates the container |
| `PUT` | `/api/v1/servers/{instance_id}/cpu` | `games.manage` | `cpu_millis` (0, or 250–128000) | Changes the CPU cap and recreates the container |
| `PUT` | `/api/v1/servers/{instance_id}/ports` | `games.manage` | `ports`: up to 16 of `port` (1024 or higher), `protocol` (`tcp`, `udp`, `both`), optional `label` (up to 40 characters) | Sets extra published ports |
| `PUT` | `/api/v1/servers/{instance_id}/start-on-boot` | `games.manage` | `enabled` | Sets the container restart policy without starting or stopping it |
| `PUT` | `/api/v1/servers/{instance_id}/browser-listing` | `games.manage` | `list_on_browser` | V Rising server-list visibility |
| `PUT` | `/api/v1/servers/{instance_id}/network` | `games.manage` + `network.firewall.write` | `enabled` | Adds or removes Helix-owned host firewall rules for the game port. No router changes |
| `POST` | `/api/v1/servers/{instance_id}/runtime` | `games.manage` | `version` (or null to repair), `expected_version`, `expected_build`, `confirmation_name` | Job: changes or repairs the Minecraft runtime |

## Servers: artwork

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/servers/{instance_id}/appearance` | `games.view` | | Current artwork and its revision |
| `PUT` | `/api/v1/servers/{instance_id}/appearance` | `games.manage` | `{"kind":"preset","expected_revision":N,"preset":"..."}` (`grass`, `portal`, `crystal`, `fortress`, `ember`, `ocean`) or `{"kind":"custom","expected_revision":N,"content_type":"image/png","image_base64":"..."}` | Sets a preset or uploads a PNG or JPEG icon (up to about 700 KiB encoded) |
| `DELETE` | `/api/v1/servers/{instance_id}/appearance` | `games.manage` | `expected_revision` | Returns to the default artwork |
| `GET` | `/api/v1/servers/{instance_id}/appearance/image` | `games.view` | Query `revision` | The stored icon bytes |

## Servers: marketplace

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/servers/{instance_id}/marketplace/search` | `games.view` | Query `query`, `offset`, `limit` (default 20, 1–50), `provider`, `catalog` (`content` or `modpacks`) | Search filtered to this server's game version and loader |
| `GET` | `/api/v1/servers/{instance_id}/marketplace/projects/{project_id}` | `games.view` | Query `provider` | Project and version detail |
| `PUT` | `/api/v1/servers/{instance_id}/address` | `games.manage` | `address` (string or null) | Saves or clears your own address for players, shown with a copy button |
| `POST` | `/api/v1/servers/{instance_id}/marketplace/install` | `games.manage` | `project_id`, `version_id` (or null), optional `provider`, optional `restart_server` (boolean) | Job: installs verified JARs into `plugins/` or `mods/`. With `restart_server` on a running server, restarts it, checks it starts, and puts the old files back if not |
| `GET` | `/api/v1/marketplace/modrinth/image` | `games.view`; session cookie, no CSRF | Query `path` | Same-origin proxy for Modrinth artwork |
| `GET` | `/api/v1/marketplace/curseforge/image` | `games.view`; session cookie, no CSRF | Query `path` | Same-origin proxy for CurseForge artwork |
| `GET` | `/api/v1/marketplace/curseforge/key` | `games.view` | | Whether a CurseForge API key is saved. Never returns the key |
| `PUT` | `/api/v1/marketplace/curseforge/key` | `games.manage` | `key` (24–256 characters) | Saves the key and tests it. `probe` is `ok`, `cdn_blocked` or `unreachable`; only a rejected key undoes the save |
| `DELETE` | `/api/v1/marketplace/curseforge/key` | `games.manage` | No body | Removes the key |

## Servers: backups

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `GET` | `/api/v1/servers/{instance_id}/backups` | `games.view` | | Backups, trash, keep rules and trash rules |
| `POST` | `/api/v1/servers/{instance_id}/backups/{backup_id}/download` | `games.backups.manage` | `{"action":"stat"}`, or `{"action":"download","offset":N,"length":N,"expected_revision":"..."}` with `length` up to 1 MiB | Exports the backup archive in chunks. Two requests at a time |
| `POST` | `/api/v1/servers/{instance_id}/backups/{backup_id}/restore` | `games.backups.manage` | No body | Job: stops the server, takes a safety backup, restores, and starts it again if it was running |
| `DELETE` | `/api/v1/servers/{instance_id}/backups/{backup_id}` | `games.backups.manage` | No body | Moves a backup to the backup trash |
| `POST` | `/api/v1/servers/{instance_id}/backups/trash/{trash_id}/restore` | `games.backups.manage` | No body | Takes a backup back out of the trash |
| `DELETE` | `/api/v1/servers/{instance_id}/backups/trash/{trash_id}` | `games.backups.manage` | No body | Deletes one trashed backup forever |
| `POST` | `/api/v1/servers/{instance_id}/backups/trash/purge` | `games.backups.manage` | `older_than_days` (0–3650; 0 empties the trash) | Deletes trashed backups deleted more than that many days ago |
| `PUT` | `/api/v1/servers/{instance_id}/backup-policy` | `games.backups.manage` | `keep_count` (0–50), `keep_days` (0–365), optional `trash_days` (0–365) | Saves keep rules and the trash rule, then applies them |
| `POST` | `/api/v1/servers/{instance_id}/backup-policy` | `games.backups.manage` | No body | Applies the saved keep rules now |

The backup list fields and the download protocol are in
[Automation API: Backups](https://github.com/Riqqqque/Helix/wiki/Automation-API#backups).
See [Backups](https://github.com/Riqqqque/Helix/wiki/Backups).

## Servers: copying, migration and removal

| Method | Path | Capability | Body or query | Does |
| --- | --- | --- | --- | --- |
| `POST` | `/api/v1/servers/{instance_id}/transfer/preflight` | `games.manage` | `target_id`, `parts` | Shows what copying `parts` from this server to the target would do. Changes nothing |
| `POST` | `/api/v1/servers/{instance_id}/transfer` | `games.manage`; also `games.backups.manage` when `parts` includes `plugin_data` or `worlds` | `target_id`, `parts`, `confirmation_name` (the target's exact name), optional `remove_missing_jars` | Job: copies the parts onto another Helix server with a backup and rollback |
| `POST` | `/api/v1/servers/migrate/preflight` | `games.manage` | `{"kind":"amp","instance_id":"..."}` or `{"kind":"folder","path":"..."}` | Inspects an AMP instance or a folder under the managed roots. Never writes the source |
| `POST` | `/api/v1/servers/migrate` | `games.manage`; also `network.firewall.write` for `public` exposure | `source`, `name`, optional `game`, `software`, `version`, `memory_mb`, optional `cpu_millis`, `max_players`, optional `game_port`, `query_port`, `network_exposure`, `start_on_boot`, `eula_accepted`, `source_stopped` (true), `copy_acknowledged` (true), optional `list_on_browser` | Job: copies a stopped source into a new native server. The source is left untouched |
| `POST` | `/api/v1/servers/{instance_id}/remove` | `games.manage` | `confirmation_name` (the exact server name) | Stops and removes the workload and moves its data to **Removed servers** |
| `GET` | `/api/v1/servers/removed` | `games.view` | | Removed servers you can restore, and the retention rule |
| `POST` | `/api/v1/servers/removed/{trash_id}/restore` | `games.manage` | No body | Restores a removed server |
| `DELETE` | `/api/v1/servers/removed/{trash_id}` | `games.manage` | `confirmation_name` | Deletes a removed server forever: world files, backups and console history. Allowed up to 300 seconds |

`parts` values are listed in
[Automation API: Copying between servers](https://github.com/Riqqqque/Helix/wiki/Automation-API#copying-between-servers).
See [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers)
and [Server Migration](https://github.com/Riqqqque/Helix/wiki/Server-Migration).

## Route count

This page lists 171 method and path pairs: `/healthz` plus 170 under `/api/v1`,
including the three automation routes covered in the
[Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API). The
router in
[`crates/helix-api/src/lib.rs`](https://github.com/Riqqqque/Helix/blob/main/crates/helix-api/src/lib.rs)
and the route modules beside it are the source of truth.
