# Server API tokens

Open **Settings → Server API tokens → Manage tokens**. Choose a name, exact
servers, permissions and an expiration of 1–90 days. Save the secret when it
appears: Helix stores only its domain-separated SHA-256 verifier. The list
never returns the secret. Revoke a token from the same card; future requests
fail immediately. Already accepted jobs are not cancelled by revocation.

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
| `view` | `server_capabilities`, `server_detail` |
| `logs` | `server_logs`, `server_log_history` |
| `files.read` | `server_files`: list, stat, read, download |
| `files.write` | `server_files`: create, write, mkdir, move, trash, all upload steps |
| `start`, `stop`, `restart`, `kill` | Matching `server_action` only |
| `console` | `server_console` |
| `settings` | `server_settings`, `update_server_settings`, `set_native_memory`, `set_native_cpu`, `set_native_start_on_boot`, `set_native_browser_listing` |
| `update` | `change_native_runtime`, server_action update |
| `backups.read` | `list_backups`, `server_backup_download` |
| `backups.write` | server_action backup, `restore_backup`, `trash_backup`, `restore_trashed_backup`, `set_backup_policy`, `prune_backups` |

Each operation takes the same typed fields as its server route, plus
`operation` and `instance_id`. File and backup downloads put their command in
`request`. Server IDs must match the token exactly, including the manager prefix.
No wildcards, host operations, firewall changes, server creation/removal,
global inventory, or token administration are delegated. Marketplace operations
are not delegated in this version; upload vetted plugin/mod files instead.

`GET /api/v1/automation/jobs` lists the last 256 jobs accepted through that token.
Poll with `{"operation":"job_status","job_id":"..."}`. A token cannot query
another token's jobs. A successful submission is not proof of job completion.
On timeout, inspect jobs and server state; never blindly replay a mutation.
Job mappings persist across dashboard restarts. Upload staging remains subject
to the broker's existing expiration and restart rules.

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

The client does not log secrets, follow redirects, use ambient HTTP proxies,
or automatically retry mutations. Uploads/downloads and cross-server transfers
reuse the bounded, checksummed client helpers. Both servers must be in scope
for a transfer. File reads can reveal plugin/RCON secrets; file writes, console
commands and software updates can substantially change a server. Grant these
only to trusted tools, not arbitrary downloaded scripts.

## Persistence and limits

Tokens and job mappings live in the critical-state database, covered by Helix
backups. Password/account authorization changes invalidate existing tokens.
Logout alone does not revoke them. At most 256 live tokens can exist; revoked
and expired records are cleaned when another token is created. Each token has
up to 64 exact server IDs, up to 90 days validity, and 600 requests per minute.
The automation request endpoint admits two concurrent broker requests.

Audit records identify the token, server, permission decision and result;
they do not retain token secrets, file contents or console command text.
A restored old database may restore credentials that were valid at backup time:
after a security incident, revoke tokens or change the owner password again.
Lost create responses cannot recover a token secret: revoke that entry and
create another. Do not give automation an owner session as a fallback.
