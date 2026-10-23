# Server API Tokens

A server API token lets a script or a build pipeline manage chosen game servers
without your dashboard login. Each token lists the exact servers it may touch
and what it may do there. Tokens never reach host controls, other servers, or
token administration.

Tokens are used with the [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API).
For the wider picture, see [API and Integrations](https://github.com/Riqqqque/Helix/wiki/API-and-Integrations).

> [!CAUTION]
> Helix is a private-LAN release. Keep the dashboard on a network you control
> and do not expose it to the internet, even with tokens.

## Create a token

1. Open **Settings → Server API tokens → Manage tokens**.
2. Name the token after the tool that will use it, for example `Plugin deploys`.
   Names are 1 to 80 characters.
3. Pick **Expires after**: 1 day, 7, 30, 90 or 180 days, 1 year, or
   **Never (until revoked)**.
4. Tick the servers it may manage. **Select all** ticks every server. Each
   server shows its exact ID (`helix:…`) under its name.
5. Leave **Full access** on, or turn it off and tick individual permissions.
6. Choose **Create token** and copy the value.

To create a token you need `system.settings.write`, `games.manage`, and
`games.view` on your own account. Some permissions also need your own rights,
because you cannot hand out more than you have:

| Token permission | Also needs on your account |
| --- | --- |
| `backups.read` or `backups.write` | `games.backups.manage` |
| `network` | `network.firewall.write` |
| Full access (`all`) | `games.backups.manage` and `network.firewall.write` |

Helix checks every server ID against the server manager before it issues the
token. A token holds up to 64 servers. A request that breaks a rule (empty
name, unknown permission, too many servers, an expiry outside 1 to 365 days)
is refused with `403 authorization_denied`.

## Permissions

Permissions apply to every server on the token. You cannot give one server
`files.write` and another only `view` on the same token; make two tokens.

| Permission | Label | Lets the token |
| --- | --- | --- |
| `all` | Full access | Everything below on the token's servers, including permissions added in later Helix versions. Never host controls. |
| `view` | View | Read status, details and capabilities; search the marketplace and read project details |
| `logs` | Logs | Read recent console output and paged log history |
| `files.read` | Read files | List, stat, read and download server files; list staged changes |
| `files.write` | Write files | Create, write, make folders, move, trash, upload (every upload step), discard staged changes; install marketplace plugins and mods |
| `start` | Start | Start the server |
| `stop` | Stop | Stop the server cleanly |
| `restart` | Restart | Restart the server, and run `deploy_pending_changes` |
| `kill` | Kill | Force-stop the server |
| `console` | Console | Run console commands |
| `settings` | Settings | Read and change settings, memory, CPU, extra ports, start on boot, server-list visibility; Valheim status and settings |
| `update` | Update | Update or repair the server software, change the runtime version; Valheim mod lookups, installs, removals and game updates |
| `backups.read` | Read backups | List and download backups |
| `backups.write` | Manage backups | Back up, restore, move to trash, restore from trash, delete forever, empty the trash, set keep and trash rules, apply keep rules |
| `network` | Network | Open or close the server's game port in the host firewall |
| `remove` | Remove server | Move the server to **Removed servers**, where it can be restored |

The [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API#operations)
lists every operation and the permission it checks.

Copying a setup between two servers needs a token that lists **both** servers,
with `files.read` plus `view` for the preview, or `files.read` plus
`files.write` for the copy itself. See
[Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers).

## Expiry

A token with a fixed lifetime stops working when it expires. A token set to
**Never** works until you revoke it or it is invalidated.

Changing your dashboard password or your account invalidates every token you
issued. They show as *Invalid — your account or password changed*, stop
working, and cannot be rotated. Create new tokens afterwards. Logging out does not affect
tokens.

## View a token again

Choose **View token**, enter your dashboard password, and Helix shows the value
with a **Copy** button. A wrong password shows *That password is not correct*
and does not sign you out.

Helix checks tokens with a one-way, domain-separated SHA-256 fingerprint. It
keeps a separate encrypted copy only so you can view the value again. That copy
is encrypted with XChaCha20-Poly1305 under a key kept in `secrets/` in the Helix
data folder, outside the `state/` database folder. A copied database or a Helix
state backup therefore does not reveal your tokens. Every view is written to
the audit log as `api_token.revealed`.

If Helix could not store the encrypted copy, the create screen says so: copy
the value right away, because it is shown once. Tokens made before Helix 1.6.0
have no viewable copy either. In both cases choose **Rotate** once to get a
value you can view any time.

## Rotate

**Rotate** issues a new value for the same token. It keeps the token's name,
servers, permissions and expiry, and clears **Last used**. The old value stops
working immediately, so update the tool that uses it at the same time. Rotating
needs the same account permissions as creating that token.

## Revoke

**Revoke** stops the token immediately. Jobs it already started keep running to
the end. Revoked tokens stay in the list, marked *Revoked*.

Up to 256 live tokens can exist. Revoked and expired records are cleaned up when
you create another token.

## The token list

Each card shows the token's status (active, expired, revoked, or invalidated by
an account or password change), its servers and permissions, when it was
created, and when it was last used. **Last used** updates at most once a
minute.

## Audit

Every automation request is written to the Helix audit log with the token ID,
the server and the result. You will see these events:

| Event | Outcome | Written when |
| --- | --- | --- |
| `api_token.<permission>` | `success` or `denied` | Helix decides whether the token may run the operation |
| `api_token.transfer` | `success` or `denied` | Helix decides on a server-to-server copy |
| `api_token.unsupported` | `denied` | The operation is not one tokens may use, or `job_status` names another token's job |
| `api_token.result` | `success` or `error` | The operation finished |
| `api_token.revealed` | `success` | You viewed a token with **View token** |

The audit log never stores the token value, file contents, or console command
text. See [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery).

## Use a token from a script

Pass the token through an environment variable. Do not paste it into a chat,
commit it, put it in a URL, or save it in a file the script reads. The commands
below ask for the token without echoing it and without saving it in your shell
history.

On Windows PowerShell:

```powershell
$s = Read-Host "Helix token" -AsSecureString
$env:HELIX_SERVER_TOKEN = [Net.NetworkCredential]::new('', $s).Password
Remove-Variable s
$env:HELIX_URL = "http://192.0.2.10:3100"
```

On Linux or macOS:

```bash
read -rs -p "Helix token: " HELIX_SERVER_TOKEN && echo && export HELIX_SERVER_TOKEN
export HELIX_URL="http://192.0.2.10:3100"
```

The variable lives only in that terminal session. Run your script from the same
window. Replace the address with your own dashboard address.

Then send it as a bearer token:

```bash
curl -sS "$HELIX_URL/api/v1/automation/server" \
  -H "Authorization: Bearer $HELIX_SERVER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"operation":"server_capabilities","instance_id":"helix:EXACT-UUID"}'
```

Do not send a `Cookie` or `Origin` header with a token; Helix refuses mixed
credentials. The server page's **Advanced** tab shows each server's exact ID.
The [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API) has
complete examples in curl, Python and PowerShell.

## Limits

- Up to 256 live tokens, each with up to 64 servers.
- 600 requests per minute per token. Over the limit Helix returns
  `429 too_many_attempts`.
- Two automation requests are processed at a time across all tokens; more wait
  for a slot.
- Revoking or rotating does not cancel jobs that were already accepted.

## Safe handling

- Give each tool its own token, with only the servers and permissions it needs.
- Use HTTPS or an SSH tunnel when you can. The bundled Python client allows
  plain HTTP for tokens only on loopback.
- File access can reveal plugin passwords and RCON settings. Console, write,
  update and Full access can change a server a lot. Give them only to tools you
  trust.
- If a token leaks, revoke it first, then create a new one.
- Restoring an old Helix state backup can bring back tokens that were valid at
  that time. After a security incident, revoke tokens again or change your
  password, which invalidates all of them.
- A lost create response cannot recover the original value unless it was stored
  for viewing. Rotate to issue a replacement.
- Never give a script your owner login as a fallback.
