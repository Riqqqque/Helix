# Server API tokens

Open **Settings → Server API tokens → Manage tokens**. Choose a name, exact
servers, permissions and an expiration of 1–365 days or **Never**. A never-expiring
token stays valid until revoked or the owner account authorization changes.
Helix authenticates tokens by a domain-separated SHA-256 verifier. It also keeps
an encrypted copy so the owner can see a token again: **View token** asks for the
dashboard password and shows the value (audited as `api_token.revealed`). The copy
is encrypted with XChaCha20-Poly1305 under a key stored in `secrets/` in the data
directory, outside `state/`, so a copied state database or state backup does not
reveal tokens. Tokens created before 1.6.0 have no copy; **Rotate** once to get a
viewable value. Rotate replaces the old secret immediately while keeping its
server scope, permissions, and expiry; update any clients using the old value. Revoke a token from the same card; future requests fail immediately.
Already accepted jobs are not cancelled by revocation or rotation.

Each token in the list shows whether it can still authenticate (active, expired,
revoked, or invalidated by an owner account/password change), its servers and
permissions, when it was created, and when it was last used.

Use HTTPS or an SSH tunnel. Do not send tokens over an untrusted HTTP network,
put them in URLs, commit them, or paste them into chat. The Python token client
allows plain HTTP only on loopback, for an SSH tunnel or local use.

## Headless requests

Send `Authorization: Bearer <token>` and `Content-Type: application/json` to
`POST /api/v1/automation/server`. Do not send Cookie or Origin headers. Browser
routes still require their session and CSRF proof; tokens cannot use those
routes. This is not an OAuth authorization server.

```json
{"operation":"server_action","instance_id":"helix:EXACT-UUID","action":"stop"}
```

```json
{"operation":"server_files","instance_id":"helix:EXACT-UUID","request":{"action":"list","path":"","limit":50}}
```

The same [relative file commands](SERVER-API.md) and safety checks apply.
Stop the selected server before publishing changes. Transfer chunks can be
staged while it runs; finishing a write needs a stopped server. Native game
support is unchanged; imported managers retain their actual adapter limits.

| Permission | Allowed operations |
| --- | --- |
| `all` (Full access) | Every permission below, including ones added later, on the token's servers |
| `view` | `server_capabilities`, `server_detail`, `server_marketplace_search`, `server_marketplace_project` |
| `logs` | `server_logs`, `server_log_history` |
| `files.read` | `server_files`: list, stat, read, download |
| `files.write` | `server_files`: create, write, mkdir, move, trash, all upload steps; `install_server_marketplace_content` |
| `start`, `stop`, `restart`, `kill` | Matching `server_action` only |
| `console` | `server_console` |
| `settings` | `server_settings`, `update_server_settings`, `set_native_memory`, `set_native_cpu`, `set_native_extra_ports`, `set_native_start_on_boot`, `set_native_browser_listing` |
| `update` | `change_native_runtime`, server_action update |
| `backups.read` | `list_backups`, `server_backup_download` |
| `backups.write` | server_action backup, `restore_backup`, `trash_backup`, `restore_trashed_backup`, `set_backup_policy`, `prune_backups`, `purge_backup_trash` |
| `network` | `set_server_network_exposure` (open or close the game port to the internet) |
| `remove` | `trash_native_server` (moves the server to Removed servers; restorable) |

Each operation takes the same typed fields as its server route, plus
`operation` and `instance_id`. File and backup downloads put their command in
`request`. Server IDs must match the token exactly, including the manager prefix.
No wildcards, host operations, host firewall rules, server creation, global
inventory, or token administration are delegated. Creating a token with `network`
or Full access requires the owner's firewall permission.

## Moving a setup to another server

`server_transfer_preflight` and `transfer_server_content` copy content from the
token's source server (`instance_id`) onto another **Helix-managed** server, for
example promoting a finished test setup to production. The token must list both
servers, with `files.read`, plus `view` for the preflight or `files.write` for the
transfer. AMP or other imported servers cannot receive transfers; import them
into Helix first.

```json
{"operation":"server_transfer_preflight","instance_id":"helix:TEST-UUID","target_id":"helix:PROD-UUID","parts":["plugins","configs"]}
```

```json
{"operation":"transfer_server_content","instance_id":"helix:TEST-UUID","spec":{"target_id":"helix:PROD-UUID","parts":["plugins","configs"],"confirmation_name":"Production","remove_missing_jars":false}}
```

| Part | Copies |
| --- | --- |
| `plugins` | Plugin JARs and config files. Leaves databases, logs, caches, and player folders (`userdata`, `playerdata`, `data`, …) behind |
| `plugin_data` | Everything under `plugins/`, including databases and player files |
| `mods` | `mods/*.jar` |
| `configs` | `bukkit.yml`, `spigot.yml`, `paper`/`purpur`/`pufferfish`/`leaves` files, `config/`, `defaultconfigs/` |
| `server_properties` | Gameplay settings. The target keeps its ports, RCON, `server-ip`, `level-name`, seed, player limit, MOTD, and whitelist settings |
| `datapacks` | The world's `datapacks/` folder |
| `player_lists` | `whitelist.json`, `ops.json`, ban lists (replaces the target's) |
| `worlds` | The world, nether, and end folders (replaces the target's worlds) |

The preflight changes nothing and reports files, sizes, errors, and warnings.
The transfer is a job (poll `job_status`): the source is packed read-only inside a
locked-down container as its own user, the target gets a full backup, stops, has
the files written without following links, and starts again if it was running.
If anything fails, including the target not starting, Helix restores the backup.
Files are added or replaced; nothing else on the target is deleted. A plugin or
mod JAR replaces the target JAR that declares the same plugin or mod name, so
`AllFather-0.9.18.jar` replaces `AllFather-0.9.17.jar`; set `remove_missing_jars`
to also delete target JARs the source no longer has. Plugins only move between
plugin servers, mods only between the same loader, and worlds or datapacks never
move onto an older Minecraft version.

`GET /api/v1/automation/jobs` lists the last 256 jobs accepted through that token.
Poll with `{"operation":"job_status","job_id":"..."}`. A token cannot query
another token's jobs. A successful submission is not proof of job completion.
On timeout, inspect jobs and server state; never blindly replay a mutation.
Job mappings persist across dashboard restarts. Small file uploads are one
content-idempotent request. Larger uploads automatically resume by path, size,
checksum and destination revision; a broker restart starts fresh without making
partial files visible.

## Python

```python
import os
from helix_client import HelixTokenClient

client = HelixTokenClient("https://helix.example", os.environ["HELIX_SERVER_TOKEN"])
server = "helix:EXACT-UUID"
client.server_capabilities(server)
# After stopping the server and checking that it stopped:
client.upload_file(server, "plugin.jar", "plugins/plugin.jar")
client.close()
```

The client does not log secrets, follow redirects, use ambient HTTP proxies, or
retry general mutations. Its upload helper does safely retry the specifically
idempotent file-upload protocol after a lost response. Uploads/downloads and
cross-server transfers reuse the bounded, checksummed client helpers. Both
servers must be in scope for a transfer. File reads can reveal plugin/RCON
secrets; file writes, console commands and software updates can substantially
change a server. Grant these only to trusted tools, not arbitrary downloaded
scripts.

## Persistence and limits

Tokens and job mappings live in the critical-state database, covered by Helix
backups. Password/account authorization changes invalidate existing tokens.
Logout alone does not revoke them. At most 256 live tokens can exist; revoked
and expired records are cleaned when another token is created. Each token has
up to 64 exact server IDs, either 1–365 days or no automatic expiry, and 600 requests per minute.
The automation request endpoint admits two concurrent broker requests.

Audit records identify the token, server, permission decision and result;
they do not retain token secrets, file contents or console command text.
A restored old database may restore credentials that were valid at backup time:
after a security incident, revoke tokens or change the owner password again.
Lost create responses cannot recover the original token secret: rotate the
entry to issue a replacement. Do not give automation an owner session as a fallback.
