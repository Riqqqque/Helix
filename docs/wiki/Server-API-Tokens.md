# Server API Tokens

A server API token lets a script, a plugin build pipeline, or an AI assistant
manage chosen game servers without your dashboard login. Each token lists the
exact servers it may touch and what it may do there. Tokens never reach host
controls, other servers, or token administration.

## Create a token

1. Open **Settings → Server API tokens → Manage tokens**.
2. Name the token after the tool that will use it, for example `Plugin deploys`.
3. Pick **Expires after**: 1 day to 1 year, or **Never (until revoked)**.
4. Tick the servers it may manage. **Select all** ticks every server.
5. Leave **Full access** on, or turn it off and tick individual permissions.
6. Choose **Create token** and copy the value.

**Full access** covers every server permission on the selected servers,
including permissions added in later Helix versions. It is the easy choice for a
trusted tool you want to "just work". Creating a Full access token needs your own
backup and firewall permissions, because the token can use them.

## View a token again

Choose **View token** on any token, enter your dashboard password, and Helix
shows the value with a **Copy** button. A wrong password shows *That password is
not correct* and does not sign you out.

Helix checks tokens with a one-way fingerprint and keeps a separate encrypted
copy only for viewing. That copy is encrypted with XChaCha20-Poly1305 under a key
kept in `secrets/` in the Helix data folder, outside the `state/` database
folder, so a copied database or a Helix state backup does not reveal your
tokens. Every view is written to the audit log.

Tokens made before Helix 1.6.0 have no viewable copy. Choose **Rotate** once to
issue a new value you can view any time. Rotating stops the old value
immediately, so update the tool that uses it at the same time.

## Permissions

| Permission | Lets the token |
| --- | --- |
| Full access (`all`) | Do everything below, including later additions |
| View (`view`) | Read status and details, search the marketplace |
| Logs (`logs`) | Read console output and log history |
| Read files (`files.read`) | List, read, and download server files |
| Write files (`files.write`) | Upload, edit, move, and delete files; install marketplace plugins and mods |
| Start, Stop, Restart, Kill | The matching lifecycle action only |
| Console (`console`) | Run console commands |
| Settings (`settings`) | Change properties, memory, CPU, extra ports, start on boot, server-list visibility |
| Update (`update`) | Update or repair the server software |
| Read backups (`backups.read`) | List and download backups |
| Manage backups (`backups.write`) | Back up, restore, delete, empty trash, and set keep rules |
| Network (`network`) | Open or close the server to the internet |
| Remove server (`remove`) | Move the server to **Removed servers**, where it can be restored |

Copying a setup between two servers needs a token that lists **both** servers
with read and write file access. See [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers).

## Give a token to an AI assistant or script

Give the token to the tool through an environment variable. Do not paste it into
chat, commit it, put it in a URL, or save it in a file the tool reads.

On Windows, start the assistant from a PowerShell window that asks for the token
without saving it to history:

```powershell
$s = Read-Host "Helix token" -AsSecureString; $env:HELIX_SERVER_TOKEN = [Net.NetworkCredential]::new('', $s).Password; Remove-Variable s; claude
```

On Linux or macOS:

```bash
read -rs HELIX_SERVER_TOKEN && export HELIX_SERVER_TOKEN && claude
```

Point the assistant at [`docs/SERVER-TOKENS.md`](https://github.com/Riqqqque/Helix/blob/main/docs/SERVER-TOKENS.md)
for every operation and field. Tell it the dashboard address and the exact server
IDs (`helix:…`) it should manage; the server page's **Advanced** tab shows each ID.

## Making requests

Every call is `POST /api/v1/automation/server` with
`Authorization: Bearer <token>` and a JSON body. Do not send cookies or an
`Origin` header; Helix rejects mixed credentials.

```json
{"operation":"server_action","instance_id":"helix:EXACT-UUID","action":"restart"}
```

```json
{"operation":"server_files","instance_id":"helix:EXACT-UUID","request":{"action":"list","path":"plugins","limit":50}}
```

Long operations return a `job_id`. Poll it with
`{"operation":"job_status","job_id":"…"}` until it completes. A token can only
see its own jobs. If a request times out, check the job list and the server
before trying again; never blindly repeat a restart, restore, or update.

Plugin and mod JARs of any size upload through the resumable chunked upload
steps. Stop the server before publishing a replacement, then start it again.

The Python client in `examples/integrations` wraps these calls, never logs the
token, and safely resumes interrupted uploads:

```python
import os
from helix_client import HelixTokenClient

client = HelixTokenClient("https://helix.example", os.environ["HELIX_SERVER_TOKEN"])
client.upload_file("helix:EXACT-UUID", "build/MyPlugin.jar", "plugins/MyPlugin.jar")
client.close()
```

## Limits and safety

- Up to 256 live tokens, each with up to 64 exact servers.
- 600 requests per minute per token, and two requests at a time.
- Changing your dashboard password or account invalidates every token. Issue
  new ones afterwards.
- Revoking a token stops it immediately; jobs it already started still finish.
- Use HTTPS or an SSH tunnel. The Python client allows plain HTTP only on
  loopback.
- File access can reveal plugin passwords and RCON settings. Give write, console,
  and Full access only to tools you trust.
