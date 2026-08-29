# API Contract

## Status

This document describes the implemented private-LAN HTTP surface. It is not a
stable public API promise. Executable route, protocol, and frontend-adapter tests
remain authoritative when this document and code disagree.

The API is versioned under `/api/v1`. Unknown `/api` routes return JSON errors;
only non-API routes use the single-page application fallback.

## Transport boundary

Source-development configuration defaults to loopback. The container deployment
can place `helixd` behind an explicitly configured private gateway with exact
Host, Origin, and client-CIDR policy. A second constrained private entry point
can be used with an already configured Tailscale route.

The built-in Hook can install and start the exact Tailscale package on eligible
Debian/Ubuntu hosts, but it does not authenticate a tailnet or configure this
gateway entry point. Direct public-internet exposure, arbitrary forwarded-
header trust, and a supported public TLS boundary have not passed the release
gate.

All JSON responses containing protected state use `Cache-Control: no-store`.
Frontend assets are separately cacheable by their hashed names.

## Authentication and request proof

The first owner is created with a short-lived one-time local bootstrap token.
Login establishes an opaque `HttpOnly`, host-only session cookie and returns a
session-bound CSRF proof held in frontend memory.

Except for liveness, setup status, owner claim, and login, current API routes
require both:

1. the valid session cookie; and
2. the current `X-Helix-CSRF` proof.

This includes protected `GET` requests because cookies are shared across ports
on one host. Reloading the browser discards the in-memory proof and returns the
user to login rather than accepting cookie-only reads.

State-changing requests additionally validate the configured Origin and Fetch
Metadata before authorization and body mapping. A valid session with a missing,
malformed, stale, or wrong proof returns `403` with code `csrf_rejected`.

## Implemented routes

### Supervisor, setup, and account

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/healthz` | Bodyless `204` supervisor liveness; no database or host detail |
| `GET` | `/api/v1/setup/status` | Bounded first-owner state |
| `POST` | `/api/v1/setup/owner` | Single race-safe owner claim |
| `POST` | `/api/v1/auth/login` | Password login |
| `GET` | `/api/v1/auth/me` | Current protected user/session state |
| `POST` | `/api/v1/auth/csrf` | Compare-and-swap CSRF rotation |
| `POST` | `/api/v1/auth/account` | Change owner login/display name and optionally password; requires `users.manage` |
| `POST` | `/api/v1/auth/logout` | Revoke the current session and clear its cookie |

### Dashboard and host reads

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/health` | `system.view` | Protected dependency health |
| `GET` | `/api/v1/system/overview` | `system.view` | Bounded CPU, memory, storage, and network snapshot |
| `GET` | `/api/v1/host/inventory` | `system.view` | Disks, mounts, interfaces, routes, services, processes, process count, thread count, CPU model, and listeners |
| `GET` | `/api/v1/weather` | `dashboard.customize` | Bounded weather data for one validated location |
| `GET` | `/api/v1/settings/preferences` | `dashboard.customize` | Revisioned dashboard preferences |
| `PUT` | `/api/v1/settings/preferences` | `dashboard.customize` | Save navigation, hidden pages, metric cadence, Home widgets, and whether the Servers page is enabled, with an expected revision |

Preferences are bounded, strictly validated, and conflict rather than silently
overwriting another session's newer revision.

### Host integration and power

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/host/integration` | `system.view` | Docker/broker service state, exact Helix container policies, Helix-only resources/errors, reboot preflight/state |
| `PUT` | `/api/v1/host/integration/start-on-boot` | `system.settings.write` | Set restart policy on the exact configured dashboard and gateway containers |
| `GET` | `/api/v1/host/reboot/preflight` | `system.view` | Active players, native/AMP servers, jobs, and blockers |
| `PUT` | `/api/v1/host/reboot/recurring` | `system.power` | Create or replace one daily/weekday local-time reboot schedule |
| `DELETE` | `/api/v1/host/reboot/recurring` | `system.power` | Remove the exact recurring reboot schedule |
| `POST` | `/api/v1/host/reboot` | `system.power` | Schedule a whole-host reboot after 10–300 seconds |
| `DELETE` | `/api/v1/host/reboot/{operation_id}` | `system.power` | Cancel the exact scheduled reboot when still cancellable |

Start-on-boot never enables/disables Docker, changes unrelated containers, or
stops/starts the current runtime. Docker restart metadata survives daemon/host
restart, but a later container recreation may reset it from Compose.

Reboot scheduling requires the exact current hostname and an explicit disruption
acknowledgement. The broker performs active-player/running-job preflight and uses
an opaque operation ID with a systemd transient timer. Recurring schedules store
one verified daily/weekday local-time plan and return the effective host timezone
and next activation. Automated tests never execute a reboot.

### Docker, Portainer, and Homarr

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/docker/inventory` | `system.view` | All Docker containers on the host, with CPU/memory when Docker reports them, plus a Portainer hint |
| `POST` | `/api/v1/docker/actions` | `system.settings.write` | Start, stop, or restart one named container after typing that exact name |
| `GET` | `/api/v1/docker/homarr` | `dashboard.customize` | Import Homarr http(s) shortcuts from classic JSON or a SQLite app catalog |

Helix talks to the Docker engine on the host. It does not proxy Portainer’s API.
Empty published-port strings are valid; one unreadable container row is skipped
instead of failing the whole list. Open Portainer uses a published port when a
Portainer container is detected.
Dashboard and gateway container names stay protected. Homarr import reads the
container bind or volume mounts, then classic JSON if present, otherwise a
read-only snapshot of `db/db.sqlite` when the `app`/`apps` table has `name` and
`href`. Only http(s) addresses are returned. Homarr board layout (`item` /
`item_layout`) is used for order and tile width when those tables exist. Icon
names and slugs are mapped onto the public dashboard-icons CDN; http(s) icon
URLs are kept. Uploaded Homarr media files, empty hrefs, and MySQL/Postgres
Homarr catalogs fail closed.

### Security center

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/security` | `system.view` | Observed and writable host/Helix security controls with explanations |
| `POST` | `/api/v1/security/controls` | `system.settings.write` | Flip one writable control after typing its exact confirmation phrase |

Writable controls are Helix start-after-boot, Minecraft auto-forward on create,
unattended-upgrades, Fail2ban, and systemd-timesyncd when those units exist.
UFW disable is not a switch here. CSRF, LAN bind, AppArmor, SSH settings, ASLR,
sysctl facts, and Docker live-restore are reported, not casually toggled.

### Storage and file operations

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/files?path=...` | `storage.files.read` | List one configured-root directory |
| `POST` | `/api/v1/files/read` | `storage.files.read` | Read a bounded supported text file |
| `POST` | `/api/v1/files/directory` | `storage.files.manage` | Create one directory |
| `POST` | `/api/v1/files/file` | `storage.files.manage` | Create one file |
| `POST` | `/api/v1/files/write` | `storage.files.manage` | Revision-guarded text write |
| `POST` | `/api/v1/files/rename` | `storage.files.manage` | Rename one entry inside its trusted root |
| `POST` | `/api/v1/files/trash` | `storage.files.manage` | Move one entry into configured recoverable trash |
| `POST` | `/api/v1/files/upload/begin` | `storage.files.manage` or `games.manage` | Start a bounded chunked upload into a writable folder or the custom-JAR import root |
| `POST` | `/api/v1/files/upload/chunk` | matching upload purpose | Append one sequential base64 chunk, maximum 2 MiB decoded |
| `POST` | `/api/v1/files/upload/finish` | matching upload purpose | Commit the exact expected size into a create-new destination |
| `POST` | `/api/v1/files/upload/abort` | matching upload purpose | Discard an in-flight temp file |
| `POST` | `/api/v1/storage/analysis` | `storage.analyze` | Start a bounded `quick` or explicit `thorough` background size analysis |
| `GET` | `/api/v1/storage/analysis/{job_id}` | `storage.analyze` | Read analysis progress/result |
| `DELETE` | `/api/v1/storage/analysis/{job_id}` | `storage.analyze` | Request cancellation |

The browser sends paths, but the broker accepts them only under configured
trusted roots and performs Linux path-safety checks. This is not an arbitrary
root filesystem API. Folder drops are rejected. Storage uploads are 1 byte–256
MiB and need `storage.files.manage`. Custom JAR uploads are 16 KiB–768 MiB ZIP
archives, land in Helix's private import root, and need `games.manage`. Chunks
travel as JSON base64 so they stay inside the existing CSRF and 5 MiB body
limit. Overwrites, out-of-order offsets, mixed purposes, and more than two
concurrent uploads fail closed. Hidden `.helix-upload-*` temps are omitted from
listings until finish or abort. An upload that sits idle for 10 minutes without
a chunk is discarded; chunks themselves reset that timer.

Quick scans cap at 30 seconds/250,000 entries; thorough
scans cap at 10 minutes/5,000,000 entries and one concurrent job. A completed
scan considers every eligible entry even though only the bounded largest
rankings are retained. Coverage, skipped entries, omitted ranking rows, and stop
reason are separate response fields. Cancellation is best effort at safe
checkpoints. File and folder rankings use allocated filesystem blocks while the
response retains logical byte lengths for comparison.

### Network and firewall

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/network/inventory` | `network.firewall.read` | Private IPv4, bounded UPnP router state, local listeners, Docker publications, game ports, owned mappings, and UFW state |
| `GET` | `/api/v1/network/globe` | `system.view` | Country-level origin and aggregated public TCP destinations. No remote addresses. |
| `POST` | `/api/v1/network/amp-router-forwards/release` | `games.manage` + `network.firewall.write` | Delete a leftover AMP-described UPnP mapping after typing `REMOVE AMP FORWARD {port}`. Refused when AMP instance files still list the port or Helix owns public access on it. AMP files are not changed |
| `POST` | `/api/v1/network/firewall/rules` | `network.firewall.write` | Create a named TCP/UDP single-port or bounded-range UFW allow rule |
| `DELETE` | `/api/v1/network/firewall/rules/{rule_id}` | `network.firewall.write` | Delete the exact Helix-owned rule into bounded Undo state |
| `POST` | `/api/v1/network/firewall/rules/{rule_id}/restore` | `network.firewall.write` | Restore the exact deleted rule before expiry |
| `POST` | `/api/v1/network/firewall/enable` | `network.firewall.write` | Confirmed activation after preserving an observed listening SSH TCP port |

Inventory keeps these facts separate:

- a local TCP/UDP listener;
- a Docker publication and host bind address;
- an active UFW matching allowance; and
- externally tested reachability.

Helix can distinguish an absent mapping, a router-confirmed Helix-owned TCP
mapping, CGNAT/non-public WAN space, and unavailable UPnP. A confirmed router
mapping still returns `reachable: null` and
`tested_from_external_network: false`: Helix does not turn a same-LAN check into
fake outside proof. Docker DNAT may not follow the normal UFW INPUT path, so a
Docker publication, UFW rule, router mapping, and outside test remain separate.

Rule writes are available only when UFW is installed, active, and its state is
verified. Helix creates exact UUID-commented allow rules with durable ownership
metadata. The separate inactive-UFW activation endpoint requires the literal
confirmation `ENABLE UFW`, proves the supplied TCP SSH port is listening, stages
an exact durable allow rule, verifies both the active state and rule, and
attempts to return to inactive state if verification fails. Helix never resets
UFW or changes its defaults. The server-specific public-access route can create
exact TCP or UDP UPnP mappings on a same-origin private IPv4 gateway (TCP for
Minecraft/Terraria, UDP game+query for V Rising, UDP game through game+2 for
Valheim), refuses to
overwrite any existing mapping including ports AMP already has claimed, and
creates matching owned UFW rules only when UFW is already active. It cannot
bypass CGNAT or an ISP block. Live AMP claims name the instance and the AMP
clicks to change the port. Leftover AMP-described UPnP mappings (no instance
file still listing that port) can be removed with
`POST /api/v1/network/amp-router-forwards/release` after the exact confirmation
`REMOVE AMP FORWARD {port}`. Helix never rewrites AMP instance files.

### System packages

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/system/packages` | `system.packages.read` | Read installed/candidate versions, sizes, source/category, held/security/restart hints, cache age, APT simulation state, and Helix GitHub-update readiness |
| `POST` | `/api/v1/system/packages/refresh` | `system.packages.write` | Start a serialized bounded APT package-list refresh job |
| `POST` | `/api/v1/system/packages/apply` | `system.packages.write` | Start a guarded job for exact selected installed/candidate tuples |
| `GET` | `/api/v1/system/packages/jobs/{job_id}` | `system.packages.read` | Read bounded refresh/apply/Helix-update progress, result, and safe logs |
| `POST` | `/api/v1/system/helix/check` | `system.packages.read` | Refresh the GitHub latest-release check for Helix itself |
| `POST` | `/api/v1/system/helix/apply` | `system.packages.write` | Start a digest-pinned Helix source apply job for an exact `vMAJOR.MINOR.PATCH` tag |

Opening the inventory does not refresh APT lists or mutate dpkg. Refresh and
apply are separate explicit jobs. Apply rechecks current/candidate versions,
holds, download headroom, and an exact no-removal/no-new-package simulation;
preserves current conffiles; requires disruption acknowledgement plus the
literal selection confirmation; verifies final versions; and never reboots.
The response makes `rollback_claimed: false` explicit for APT. Helix self-update
is a separate control: it requires `UPDATE HELIX`, disruption acknowledgement,
and a newer digest-pinned GitHub tag. It claims rollback only for Helix
dashboard/gateway images and broker binaries. `git_pull_used` stays false.

### Hooks

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/hooks` | `system.view` | Inventory exact configured systemd services, the optional AMP adapter, and the Docker engine hook |
| `GET` | `/api/v1/hooks/{hook_id}/install/preflight` | `system.view` | Read exact host prerequisites, planned file writes, blockers, and owner steps for a built-in installer |
| `POST` | `/api/v1/hooks/{hook_id}/install` | `system.settings.write` | Start one exact allowlisted Tailscale or Jellyfin package/service installation |
| `GET` | `/api/v1/hooks/jobs/{job_id}` | `system.view` | Read bounded hook-install progress and verified result |
| `POST` | `/api/v1/hooks/{hook_id}/actions` | `system.settings.write` | Start, stop, restart, enable, or disable one exact allowlisted service and verify resulting state |

The default catalog describes Plex, Tailscale, Pterodactyl Wings, and Jellyfin;
the root-owned broker configuration remains authoritative. Eligible Debian or
Ubuntu hosts can install the exact Tailscale or Jellyfin package from its exact
official signed repository. Pterodactyl remains guided because its panel owns
the node configuration and credentials. There is no caller-supplied package,
repository, unit, executable, remote install script, upstream account login, or
claim of full upstream API parity.

### Strands

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/strands` | `system.view` | List installed UI-only packages |
| `POST` | `/api/v1/strands/inspect` | `system.settings.write` | Review a zip upload or https zip URL without installing |
| `POST` | `/api/v1/strands` | `system.settings.write` | Install or replace a package (starts disabled) |
| `PUT` | `/api/v1/strands/{id}` | `system.settings.write` | Enable or disable host calls and UI files |
| `DELETE` | `/api/v1/strands/{id}` | `system.settings.write` | Remove the package and its namespaced storage |
| `GET` | `/api/v1/strands/{id}/package` | `system.view` | Export the stored zip |
| `GET` | `/api/v1/strands/{id}/files/{*asset}` | `system.view` | Serve a sandboxed UI asset (session cookie, no CSRF) |
| `POST` | `/api/v1/strands/{id}/host` | `system.view` | Run a declared host call for an enabled Strand |

Installable packages are `helix.strand/1` UI-only zips. Host methods are
`metrics.snapshot`, `storage.get|set|delete|list`, and `net.fetch`. HTTPS is
origin-allowlisted and blocked from private/link-local/metadata addresses.
Replacing the same UUID keeps KV and sets `enabled` to false. Portable Wasm is
not a runtime. There is no Helix-operated store and no signature check;
unsigned zips are owner-authorized.

### Optional host terminal

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/terminal/status` | `terminal.open` | Report whether the separate non-root Linux PTY service socket is ready |
| `POST` | `/api/v1/terminal/ticket` | `terminal.open` | Recheck current password and set a 30-second one-use ticket cookie |
| `GET` | `/api/v1/terminal/connect` | `terminal.open` | Upgrade to the exact `helix-terminal-v1` WebSocket protocol |

The ticket is random, session-bound, path-scoped, `HttpOnly`, `SameSite=Strict`,
single-use, and never returned in a URL or JSON field. WebSocket setup requires
the session cookie, the ticket cookie, an exact same Origin, and the one expected
subprotocol. The bridge connects only to the configured Unix socket; the host
service independently checks Linux peer credentials and runs as the configured
non-root user. Helix records authorization/lifecycle events, not terminal input
or output. Disconnect ends the PTY.

### Servers and jobs

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/servers` | `games.view` | List native and separate AMP-managed instances |
| `GET` | `/api/v1/servers/inventory-health` | `games.view` | Typed AMP compatibility-inventory health, bounded unverified-instance details, and unavailable/degraded state |
| `GET` | `/api/v1/servers/manager/readiness` | `games.view` | Native manager backend, supported software, capabilities, and retention policy |
| `GET` | `/api/v1/servers/minecraft/versions?software=` | `games.view` | Bounded published Minecraft releases for one installable software choice |
| `GET` | `/api/v1/servers/port-policies/minecraft` | `games.view` | Read the normalized Minecraft ranges, priority ports, capacity, assignments, AMP-claimed numbers in the pool, and next free port |
| `PUT` | `/api/v1/servers/port-policies/minecraft` | `games.manage` | Persist bounded ranges, individual priority ports, and the public-setup default |
| `GET` | `/api/v1/servers/port-policies/vrising` | `games.view` | Read the V Rising UDP pool (game + query pairs) |
| `PUT` | `/api/v1/servers/port-policies/vrising` | `games.manage` | Persist the V Rising UDP pool and optional public-setup default |
| `GET` | `/api/v1/servers/port-policies/valheim` | `games.view` | Read the Valheim UDP pool (game + next two) |
| `PUT` | `/api/v1/servers/port-policies/valheim` | `games.manage` | Persist the Valheim UDP pool and optional public-setup default |
| `GET` | `/api/v1/servers/port-policies/terraria` | `games.view` | Read the Terraria TCP pool |
| `PUT` | `/api/v1/servers/port-policies/terraria` | `games.manage` | Persist the Terraria TCP pool and optional public-setup default |
| `GET` | `/api/v1/games/readiness` | `games.view` | Compatibility alias for manager readiness |
| `POST` | `/api/v1/servers/minecraft` | `games.manage` | Start a native Minecraft creation job |
| `POST` | `/api/v1/servers/vrising` | `games.manage` | Start a native V Rising creation job |
| `POST` | `/api/v1/servers/valheim` | `games.manage` | Start a native Valheim creation job |
| `POST` | `/api/v1/servers/terraria` | `games.manage` | Start a native Terraria creation job |
| `GET` | `/api/v1/servers/minecraft/modpacks/search` | `games.view` | Search Modrinth or CurseForge modpack previews (`provider=modrinth` or `curseforge`) |
| `GET` | `/api/v1/servers/minecraft/modpacks/projects/{project_id}` | `games.view` | Read bounded project/version compatibility detail |
| `POST` | `/api/v1/servers/minecraft/modpacks` | `games.manage` | Start a server-safe modpack creation job |
| `GET` | `/api/v1/servers/{instance_id}` | `games.view` | Native or AMP detail |
| `GET` | `/api/v1/servers/removed` | `games.view` | Recoverable removed native servers and retention policy |
| `POST` | `/api/v1/servers/removed/{trash_id}/restore` | `games.manage` | Restore an exact removed native server |
| `DELETE` | `/api/v1/servers/removed/{trash_id}` | `games.manage` | Permanently delete an exact removed native server after typing its name; wipes world files, backups, and console history |
| `POST` | `/api/v1/servers/{instance_id}/actions` | `games.manage` | Typed start/stop/restart/kill/update/backup action |
| `PUT` | `/api/v1/servers/{instance_id}/start-on-boot` | `games.manage` | Set Docker restart policy on one native game container without starting or stopping it now |
| `PUT` | `/api/v1/servers/{instance_id}/memory` | `games.manage` | Set allocated memory on one native game container; recreates the published container with the new limit |
| `PUT` | `/api/v1/servers/{instance_id}/cpu` | `games.manage` | Set Docker `--cpus` on one native game container (`cpu_millis`: `0` = no extra cap, else 250–128000); recreates the published container |
| `PUT` | `/api/v1/servers/{instance_id}/browser-listing` | `games.manage` | Set V Rising `ListOnEOS` / `ListOnSteam` / `HideIPAddress`; `restart_required` when the container is running |
| `PUT` | `/api/v1/servers/{instance_id}/network` | `games.manage` + `network.firewall.write` | Create or remove the exact verified Helix-owned TCP or UDP router/UFW exposure for a native server |
| `POST` | `/api/v1/servers/{instance_id}/remove` | `games.manage` | Stop/remove exact native workload and move data to recoverable trash |
| `GET` | `/api/v1/jobs/{job_id}` | `games.view` | Read current bounded job state/log |

The native readiness contract currently advertises install paths for Paper,
Purpur, Folia, Leaves, Fabric, Forge, NeoForge, Quilt, Pufferfish, Vanilla,
guarded local custom-JAR import, V Rising, Valheim, and Terraria when Docker is
ready. Dedicated games use Helix-owned isolated runtime images, not unpinned
Hub tags.

`GET /api/v1/servers/minecraft/versions` returns up to 128 published releases
for one software id. Paper, Folia, and Leaves hide Minecraft versions newer than
the current default/stable channel so Latest matches what create actually
installs. Paper-family and Fabric/Vanilla catalogs accept `latest`; custom JAR
catalogs return Mojang releases and reject `latest` at create time.

V Rising creation installs the dedicated server into an isolated container,
allocates a UDP game/query pair from the V Rising pool, lists on EOS/Steam by
default (`list_on_browser`, with `HideIPAddress` when listed), and may request
public UDP UPnP when `network_exposure` is `public`. The first create may build
`helix-vrising-runtime:1` and download Steam app `1829350`. Removing the last
active V Rising instance deletes that image.
Restore rebuilds it if needed. This path is implemented and unvalidated on a
live host; it is not publisher-supported.

Native start-on-boot writes Docker `--restart unless-stopped` or `no` on the
exact instance container and persists the same flag on the instance manifest.
It does not start or stop the server at toggle time. After a host reboot,
Docker brings back servers that opted in.

Native allocated memory writes the instance manifest and recreates the published
container with the new Docker memory limit. Minecraft also updates `-Xmx`. Native
CPU writes `cpu_millis` and recreates the container with Docker `--cpus`. `0`
means no extra cap. The container is started again only if it was running. Memory
bounds match create: Minecraft 1–24 GiB, V Rising 2–24 GiB, Valheim 1–16 GiB,
Terraria 512 MiB–8 GiB. CPU bounds are off, or 0.25–128 cores.

Accepted work is not completed work. Creation, install, update, and backup jobs
return bounded broker-lifetime status that the frontend polls. Job state is not
yet a crash-persistent queue. Incompatible per-instance operations are
serialized or rejected rather than run concurrently. Native `kill` is the
exception for a hung stop/restart: it uses `docker kill`, skips the exclusive
instance lock, and queues beside that lifecycle job. It is rejected during
backup, update, restore, or marketplace install. AMP `kill` is rejected; AMP
instances stay under AMP.

Minecraft creation accepts either one explicit port or no port, which allocates
the first genuinely free candidate from the stored Minecraft policy while the
creation lock is held. Priority ports are tried before ordered ranges; the
policy is bounded to 4,096 unique candidates. Modpack creation accepts opaque
project/version IDs, optional `provider` (`modrinth` default, or `curseforge`),
and the ordinary server name, RAM, optional CPU cap (`cpu_millis`, `0` omitted),
player, optional port, network-exposure,
start-on-boot, and EULA fields. Modrinth `.mrpack` downloads use exact API/CDN
hosts without redirects and verify declared hashes. CurseForge uses the public
website catalog and forgecdn files plus `manifest.json`. Fabric, Forge,
NeoForge, and Quilt loaders can be pinned. The result reports excluded
optional/client-only files and `full_pack_parity: false`.

### Server console, settings, marketplace, and backups

| Method | Route | Capability | Purpose |
| --- | --- | --- | --- |
| `GET` | `/api/v1/servers/{instance_id}/logs?lines=...` | `games.view` | Stable bounded recent-log response |
| `GET` | `/api/v1/servers/{instance_id}/logs/history?cursor=...&lines=...` | `games.view` | Paged history across retained native console archives, maximum 500 entries |
| `POST` | `/api/v1/servers/{instance_id}/console` | `games.manage` | Send one bounded native console command |
| `GET` | `/api/v1/servers/{instance_id}/appearance` | `games.view` | Read revisioned uploaded/preset server artwork state |
| `PUT` | `/api/v1/servers/{instance_id}/appearance` | `games.manage` | Set a validated preset or optimized PNG/JPEG icon |
| `DELETE` | `/api/v1/servers/{instance_id}/appearance` | `games.manage` | Return to the default server artwork |
| `GET` | `/api/v1/servers/{instance_id}/appearance/image` | `games.view` | Serve the stored same-origin icon bytes |
| `GET` | `/api/v1/servers/{instance_id}/settings` | `games.view` | Read settings, revision, field metadata, and restart state |
| `POST` | `/api/v1/servers/{instance_id}/settings` | `games.manage` | Revision-guarded settings update |
| `GET` | `/api/v1/servers/{instance_id}/marketplace/search` | `games.view` | Compatibility-filtered Modrinth or CurseForge search (`provider`, `catalog=content\|modpacks`) |
| `GET` | `/api/v1/servers/{instance_id}/marketplace/projects/{project_id}` | `games.view` | Bounded project/version detail (`provider`) |
| `POST` | `/api/v1/servers/{instance_id}/marketplace/install` | `games.manage` | Start an exact compatible content install job (files only; no restart) |
| `GET` | `/api/v1/marketplace/modrinth/image?path=...` | `games.view` | Same-origin, session-authenticated, exact-CDN-path image proxy for marketplace and modpack artwork |
| `GET` | `/api/v1/marketplace/curseforge/image?path=...` | `games.view` | Same for CurseForge `media.forgecdn.net` avatars |
| `GET` | `/api/v1/servers/{instance_id}/backups` | `games.view` | Active backups, recoverable trash, trash note, and keep-count/keep-days policy |
| `PUT` | `/api/v1/servers/{instance_id}/backup-policy` | `games.backups.manage` | Set keep-count (0–50) and keep-days (0–365); 0 means no limit; extras move to trash |
| `POST` | `/api/v1/servers/{instance_id}/backup-policy` | `games.backups.manage` | Apply the saved keep rules now |
| `POST` | `/api/v1/servers/{instance_id}/backups/{backup_id}/restore` | `games.backups.manage` | Restore an exact backup |
| `DELETE` | `/api/v1/servers/{instance_id}/backups/{backup_id}` | `games.backups.manage` | Move an exact backup into protected trash |
| `POST` | `/api/v1/servers/{instance_id}/backups/trash/{trash_id}/restore` | `games.backups.manage` | Undo an exact recoverable deletion |
| `DELETE` | `/api/v1/servers/{instance_id}/backups/trash/{trash_id}` | `games.backups.manage` | Delete a trashed backup forever |

The game console is HTTP request/response plus durable cursor history; it is
separate from the host-terminal WebSocket. Browser closure does not stop native
archive capture. Retention is
bounded by configured bytes and segment count, so “persistent” does not mean
unlimited or permanent.

`GET /api/v1/servers` and native `GET /api/v1/servers/{instance_id}` include
`tps` when AMP reports it, or when a running native Minecraft server answers
local RCON `/tps`. Otherwise `tps` is null. The value is the first 1-minute
sample from that command, not the unauthenticated status ping.

Settings identify restart-required fields and report pending restart state.
Minecraft `game_port` is included: a change rewrites `server-port` and
`query.port`, recreates the published container, and removes Helix public access
on the previous port. AMP-claimed ports are refused. Minecraft `memory_mb` is
saved on the same settings POST and also has `PUT /api/v1/servers/{instance_id}/memory`
for every native game. Overview PUBLIC INTERNET is the detected public IP and
game port plus a port-forward reminder; it does not call the public-access route.
Marketplace profiles prevent plugin/mod loader mixing and require a matching
game version and supported loader. A missing or negative Modrinth server-side
flag is advisory: the API returns it for the UI warning, but does not block an
otherwise matching JAR. Search accepts `provider=modrinth|curseforge` and
`catalog=content|modpacks`. Install writes checksum-verified JARs into
`plugins/` or `mods/` and does not restart the container. Modpack create is a
separate server-safe subset from Modrinth `.mrpack` or public CurseForge
`manifest.json` packs; it is not a full client copy and does not claim every
upstream pack. Backup list responses include `policy.keep_count` and
`policy.keep_days`. Zero means no limit. Count/age extras move to trash;
`DELETE .../backups/trash/{trash_id}` destroys that trash entry.

## Broker protocol boundary

`helixd` is unprivileged. Broker-backed routes serialize one length-bounded,
versioned `BrokerRequest` over the configured Unix socket. `helix-privd` accepts
only the closed operation enum and independently validates configured roots,
opaque IDs, exact container names, ports, paths, and state transitions.

There is no `run_command`, caller-supplied shell string, arbitrary unit name, or
caller-selected broker binary. HTTP authorization is repeated before a broker
request, while the broker enforces authority-independent safety and identity
rules.

## Resource conventions

- Stable opaque IDs appear in URLs; display names do not identify host paths.
- Timestamps end in `_unix_ms` or `UnixMs` and are UTC Unix-epoch milliseconds.
- Sizes are bytes. Full-range storage/cumulative counters may use canonical
  decimal strings when a JavaScript number would lose precision.
- Enum values are lowercase machine identifiers; frontend labels are separate.
- Unknown, null, empty, unavailable, and false keep distinct meanings.
- Lists, console pages, directory reads, analysis results, marketplace results,
  and logs have explicit caps.
- Protected resources use no-store caching.

## Errors and retries

The current bounded error shape is:

```json
{
  "code": "stable_machine_code",
  "message": "Safe text for the operator"
}
```

Some conflicts include safe current revision metadata. Stack traces, SQL,
secrets, tokens, internal command arguments, and private configured roots are
not API error fields.

Common status classes:

- `400` malformed or invalid typed input;
- `401` missing or invalid authentication;
- `403` missing capability or rejected CSRF/Origin proof;
- `404` absent or concealed object;
- `409` revision, state-machine, ownership, or operation conflict;
- `413` body exceeds its endpoint limit;
- `422` typed request cannot satisfy current domain state;
- `429` operation-specific rate/concurrency limit; and
- `503` broker/dependency unavailable or protective maintenance state.

Retryable errors include `Retry-After` where the current handler defines it.
Clients must refetch authoritative state after a mutation; they must not assume
an optimistic frontend transition succeeded.

## Input and output safety

- JSON bodies use strict typed structures and bounded endpoint-specific limits.
- Mutation headers are validated before capability-dependent body processing
  where the handler accepts fallible JSON mapping.
- Paths stay within configured roots and are revalidated by the broker.
- Commands are closed typed operations or a bounded game-console line, never a
  shell command assembled from user text.
- URLs and redirects are HTTPS/host constrained per integration.
- Downloaded marketplace/runtime artifacts are bounded and checked against the
  evidence available from their upstream contract.
- User-visible Helix chrome is escaped by the frontend. Strand UI is third-party
  HTML served into a sandboxed iframe with `connect-src 'none'` and no
  `allow-same-origin`.

## Compatibility and release limits

Within `/api/v1`, additive response fields may appear. Removing a field or
changing its meaning requires an explicit version/deprecation decision. The
private-LAN API has not completed a public compatibility period.

Still unvalidated for public support: public exposure of the Helix dashboard, arbitrary router protocols, a general TLS
and proxy deployment, independent authentication/broker/terminal review, broad
supported-Ubuntu matrices, browser reconnect events, live UFW mutation and
package-interruption matrices, independently signed Helix keys, and broad game or modpack
compatibility.
