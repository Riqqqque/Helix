# Connecting tools to Helix

Helix has a private, versioned HTTP API. Scripts, dashboards and other trusted
tools can use it without reading the database, guessing container names, or
getting access to the Docker socket.

Start with `GET /api/v1/discovery`. It returns the API version, the current
session's capabilities, links, and the authentication and retry rules. Then
fetch `GET /api/v1/openapi.json`, or use [the checked-in contract](openapi.json).
Both endpoints require authentication. They work even when the host broker is
unavailable; discovery is not a health check or a promise that hosting is ready.

The OpenAPI document lists every server route and method, with typed request
bodies. It does not yet describe every host/dashboard operation or fully type
every broker response. [Server automation API](SERVER-API.md) covers per-game
capabilities, relative files, upload/download, transfers and backup exports.
[API.md](API.md) documents the wider file, settings, marketplace, runtime,
backup and host routes. Existing response fields retain their meaning within
v1; clients should accept extra fields and fail safely on unknown job states.

## Credentials and the trust boundary

Login through `POST /api/v1/auth/login` with `loginName` and `password`.
Keep the returned session cookie and `csrfToken` together. Every protected
request needs both the cookie and `X-Helix-CSRF`. Mutations additionally need
the exact configured `Origin` and the documented content type. Do not relax
Host/Origin checks, add wildcard CORS, or copy browser cookies to work around
authentication failures. Revoke the session with `POST /api/v1/auth/logout`,
`Content-Type: application/json` and the body `{}`.

**There are no delegated API keys or per-server credentials yet.** A session
has its user's permissions. Filtering a list in a client does not restrict
those permissions. Do not give an owner login to an untrusted plugin or a tool
that should manage only one server. Discovery reports these missing features
explicitly, so a caller that needs them can refuse to connect.

Use a trusted private connection. The reference client verifies TLS, rejects
redirects, ignores ambient proxy settings and keeps credentials in memory.
Loopback HTTP is supported for local use or an SSH tunnel. Plain HTTP to a
private IP requires explicit opt-in and is still unencrypted on that network.
The dashboard's public TLS/proxy support remains a separate release gate.

## Working example

The [Python reference client](../examples/integrations/helix_client.py) uses
only Python's standard library. Run this beside that file, using your own
configured address. The example only reads inventory; it does not restart or
modify anything.

```python
from getpass import getpass
from helix_client import HelixClient

# Example: an existing SSH tunnel to the private Helix entry point.
client = HelixClient("http://127.0.0.1:3100")
client.login(input("Helix login: "), getpass("Helix password: "))
try:
    print(client.discovery()["capabilities"])
    for server in client.servers():
        print(server["id"], server["name"], server["manager"], server["status"])
finally:
    client.logout()
```

For direct private-LAN HTTP, construct it with
`HelixClient("http://192.168.1.20:3100", allow_private_http=True)` instead.
That address is an example, not an installation default. Passwords must not
be command-line arguments, source-code constants, committed files or logs.

## Choosing the right server

`GET /api/v1/servers` returns an **array**, not a `{ "servers": ... } wrapper.
Use its `id` unchanged as the `instance_id` URL parameter. Native IDs are full
UUIDs prefixed with `helix:` in HTTP inventory. Keep that prefix. The shell
inventory helper's `instance_id` is the bare UUID; do not confuse the two
transport formats. Imported IDs use their own manager prefix.

Names can collide or change. Never select the first item, infer identity from
a version or port, or assume an imported server is owned by Helix. Verify the
ID, display name, manager, software and version before a write. Check
`/servers/manager/readiness` and the selected server's details as well.

## Actions and long-running jobs

An explicit `POST /servers/{instance_id}/actions` accepts one action:
`start`, `stop`, `restart`, `kill`, `update` or `backup`. Use `stop`, not `kill`,
for ordinary shutdowns. Native operations return a `job_id`; imported-manager
operations may complete synchronously. Not every manager supports every action.

Save the returned ID. Poll `GET /jobs/{job_id}` no faster than needed—two
seconds is a reasonable starting point. `queued` and `running` are in progress;
only `complete` means success. `failed` is a terminal failure even with HTTP
200. The reference client's `wait_for_job()` handles those states and checks
that the response belongs to the requested job.

An HTTP timeout, disconnect, closed page or polling deadline **does not cancel
an accepted operation**. Do not repeat a mutation to find out whether it worked.
Check the saved job ID and server state first. Helix has some internal operation
deduplication, but it does not implement a general `Idempotency-Key` contract.
Unknown jobs, missing responses and changed broker state require reconciliation,
not automatic replay. Job retention is finite.

The client does not retry requests automatically. If adding read retries,
honor `Retry-After`, use bounded backoff and stop on authentication failures.
Errors contain `code` and `message`; branch on the code, not English text.
Keep `X-Request-ID` when reporting a failure. Do not log response bodies that
could contain secrets, file contents or console output.

## Settings, files and plugin deployment

- Read settings first and send their expected revision when saving. A conflict
  means reload and reconcile; do not overwrite another user's changes blindly.
- Treat settings save and restart as distinct operations. Read settings again
  after the restart; a successful HTTP response alone is not runtime proof.
- Prefer `/servers/{instance_id}/files` for server files: relative paths,
  revision checks, atomic replacement, recoverable trash and chunked transfers.
  The general Storage API uses host paths and is not a server-scoped sandbox.
- Resolve the exact server and verify loader/game/plugin compatibility before
  installing a JAR or mod. Back up, stop the selected server, stage and verify
  the replacement, then start and inspect its logs. Never overwrite live worlds.
- Keep operation IDs and backup IDs outside the browser when an integration
  must survive its own restart. A backup is useful only if restoration works.

See [server automation](SERVER-AUTOMATION.md) for host-side discovery and safe
file replacement. For UI extensions with declared capabilities, use
[Strands](STRAND-DEVELOPMENT.md) rather than handing the extension an owner cookie.

## Checks that protect this contract

API tests verify authentication on discovery, real capability reporting,
method/query error JSON, and that each documented method/path exists. Reference
client tests cover cookies, CSRF, Origin, redirect refusal, URL boundaries,
expired sessions, bounded responses, job states and no mutation replay.

```sh
cargo test --locked -p helix-api
python3 -m unittest discover -s examples/integrations -p 'test_*.py' -v
```

Delegated credentials, server-scoped credentials, complete typed response
coverage, webhooks and generic idempotency are not implemented by this pass.
Do not advertise them as available or weaken the current checks to imitate them.
