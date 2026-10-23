# API and Integrations

Helix has a private, versioned HTTP API under `/api/v1`. The dashboard itself
uses it, and you can use it from scripts, build pipelines and other trusted
tools without reading the database, guessing container names, or touching the
Docker socket.

> [!CAUTION]
> Helix is a private-LAN release, not a public-internet product. The API is
> served on the same private entry point as the dashboard. Keep it on a network
> you control and do not expose it to the internet.

This page explains how the API works. The two reference pages list every call:

- [Dashboard API Reference](https://github.com/Riqqqque/Helix/wiki/Dashboard-API-Reference):
  every route the signed-in owner can use.
- [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API):
  every operation a server API token can run.

## Two ways in

| | Dashboard API | Automation API |
| --- | --- | --- |
| Who uses it | The dashboard, or a script logged in as the owner | Scripts and pipelines holding a [server API token](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens) |
| Credentials | `helix_session` cookie plus the `X-Helix-CSRF` header; mutations also need `Origin` and `Content-Type: application/json` | `Authorization: Bearer <token>` only. No cookie, no `Origin` |
| Reach | Everything the account's capabilities allow: host, storage, network, packages, Docker, hooks, terminal, Strands, servers, backups | Only the servers listed on the token, only the permissions on the token |
| Routes | About 170 method and path pairs | `POST /api/v1/automation/server`, `GET /api/v1/automation/jobs`, `GET /api/v1/automation/jobs/{job_id}/wait` |
| Shape | One REST-style route per action | One endpoint; the JSON body names the `operation` |

Use a token for automation. An owner session carries every owner permission,
so a script that only deploys plugins to one server should never hold it.

## Where the API lives

In the standard container deployment the dashboard and API are on the gateway
port, `3100` by default (`HELIX_LAN_PORT`). Every path in these pages is
relative to that address, for example `http://192.0.2.10:3100/api/v1/servers`.
The examples use `$HELIX_URL` for it.

The gateway checks three things before a request reaches Helix:

- The `Host` header must be one of the configured gateway hosts, otherwise
  `421`.
- The client address must be loopback or inside the allowed client CIDRs,
  otherwise `403`.
- `POST`, `PUT` and `DELETE` must carry the configured browser `Origin`,
  otherwise `403`. The one exception is `POST /api/v1/automation/server` with
  no `Origin` and no `Cookie`, which is how token requests pass.

Behind the gateway, `helixd` listens on loopback only and refuses any `Host`
that is not `localhost` or a loopback IP (`421 untrusted_host`). That is why
you cannot reach port 8080 directly from another machine. See
[Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration)
for the gateway settings.

Unknown `/api` paths return a JSON `404`; only non-API paths fall back to the
dashboard page. `GET /healthz` is a bodyless `204` liveness check with no
authentication and no detail.

## Discovery and the OpenAPI document

Start with `GET /api/v1/discovery`. It needs a signed-in session and the CSRF
header, and returns:

| Field | Meaning |
| --- | --- |
| `schema_version` | `1` |
| `api_version` | `v1` |
| `helix_version` | The running Helix version, for example `1.10.0` |
| `openapi_url` | `/api/v1/openapi.json` |
| `contract_scope` | `server-management` |
| `capabilities` | The current session's capabilities |
| `authentication` | Session mode, CSRF header name, whether mutations need `Origin`, and the token endpoint and transport |
| `conventions` | Server identity field (`id`), timestamp unit (Unix milliseconds), `X-Request-ID`, `Retry-After`, `automatic_mutation_retry_safe: false`, `idempotency_keys_supported: false`, `content_idempotent_server_uploads: true` |
| `links` | Session, servers, readiness, and templates for capabilities, files and jobs |

`GET /api/v1/openapi.json` serves the checked-in
[OpenAPI document](https://github.com/Riqqqque/Helix/blob/main/docs/openapi.json)
(OpenAPI 3.1.1, contract version `1.0.0`). It also needs a session and the CSRF
header. Discovery and OpenAPI keep working when the host broker is down, so
neither is a health check.

The OpenAPI document covers the server-management part of the API: login and
session, discovery, server inventory, creation, lifecycle, console, files,
deploy, settings, ports, artwork, marketplace, modpacks, runtime, backups,
transfers, removal, jobs, server tokens, and the automation endpoints. It
types the request bodies. Many broker responses are still open objects. Host,
storage, network, package, hook, Docker, security, terminal, machine, Strand,
preference and weather routes are not in it yet; they are listed in the
[Dashboard API Reference](https://github.com/Riqqqque/Helix/wiki/Dashboard-API-Reference).

## Signing in as the owner

1. `POST /api/v1/auth/login` with `{"loginName": "...", "password": "..."}`.
2. The response sets the `helix_session` cookie (`HttpOnly`, `SameSite=Strict`,
   host-only) and returns `csrfToken`, `user`, `expiresAtUnixMs` and
   `sessionExpires`.
3. Send the cookie and `X-Helix-CSRF: <csrfToken>` on every later request,
   including `GET`. Helix never accepts the cookie alone.
4. On every `POST`, `PUT` and `DELETE`, also send
   `Content-Type: application/json` and the exact `Origin` of the dashboard.
   Send the content type even when the route takes no body. A
   `Sec-Fetch-Site: cross-site` header is refused.
5. When you are done, `POST /api/v1/auth/logout` with the body `{}`.

By default a session ends after 30 minutes idle or 8 hours at most.
`PUT /api/v1/auth/session-expiry` with `{"expires": false}` keeps it until you
log out or change your password, capped at 400 days.
`POST /api/v1/auth/csrf` with `{}` swaps the CSRF value for a new one.

`$HELIX_URL` must be exactly the dashboard origin the gateway was configured
with (`HELIX_BROWSER_ORIGIN`), or the gateway refuses the login.

```bash
# Log in and keep the cookie in a private file for this shell session.
jar=$(mktemp)
read -rs -p "Helix password: " HELIX_PASSWORD && echo
csrf=$(jq -n --arg p "$HELIX_PASSWORD" '{loginName: "owner", password: $p}' |
  curl -sS -c "$jar" "$HELIX_URL/api/v1/auth/login" \
    -H "Content-Type: application/json" -H "Origin: $HELIX_URL" -d @- | jq -r .csrfToken)
unset HELIX_PASSWORD

curl -sS -b "$jar" -H "X-Helix-CSRF: $csrf" "$HELIX_URL/api/v1/servers"

curl -sS -b "$jar" -X POST "$HELIX_URL/api/v1/auth/logout" \
  -H "X-Helix-CSRF: $csrf" -H "Content-Type: application/json" \
  -H "Origin: $HELIX_URL" -d '{}'
rm -f "$jar"
```

The [Dashboard API Reference](https://github.com/Riqqqque/Helix/wiki/Dashboard-API-Reference#authentication-and-capabilities)
lists the capabilities each route checks. The owner holds all of them.

## Signing in with a token

Create a token in **Settings → Server API tokens** and send it as
`Authorization: Bearer <token>`. Do not send a `Cookie` or an `Origin` header;
Helix refuses requests that mix credentials. Tokens cannot call the dashboard
routes, discovery, or the OpenAPI endpoint.

```bash
curl -sS "$HELIX_URL/api/v1/automation/server" \
  -H "Authorization: Bearer $HELIX_SERVER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"operation":"server_detail","instance_id":"helix:EXACT-UUID"}'
```

See [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)
for permissions and handling, and the
[Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API) for every
operation.

## Errors

Every API error is JSON with a stable machine code and a safe message:

```json
{
  "code": "authorization_denied",
  "message": "The session is not authorized for this operation."
}
```

Branch on `code`, never on the English text. When the host broker refuses an
operation (a stale revision, a full disk, a server that is busy, a missing
file), the response is `400` with code `broker_operation_rejected` and a
message that names the cause, for example:

```json
{
  "code": "broker_operation_rejected",
  "message": "file revision conflict; reread the file and reconcile before changing or downloading it"
}
```

Errors never include stack traces, SQL, secrets, tokens, internal command
arguments, or private host paths.

| Status | Common codes | Meaning |
| --- | --- | --- |
| `400` | `invalid_json`, `invalid_request`, `broker_operation_rejected`, and route-specific codes | The body or query is malformed, or the broker refused the operation |
| `401` | `authentication_required`, `login_rejected`, `current_password_rejected`, `terminal_ticket_rejected` | Missing or invalid credentials, or a password recheck failed |
| `403` | `authorization_denied`, `csrf_rejected`, `invalid_origin`, `cross_site_request_rejected` | Missing capability or token permission, or the request proof was refused |
| `404` | `api_route_not_found`, `not_found` | Unknown route, or an object that does not exist or is hidden |
| `405` | `method_not_allowed` | Wrong HTTP method for this path |
| `408` | `request_timeout` | Helix stopped waiting for the handler. An accepted operation may still be running |
| `409` | `account_update_conflict`, `server_appearance_conflict`, `strand_conflict`, `setup_conflict` | Someone else changed the thing first |
| `413` | `payload_too_large` | The body is over this route's limit |
| `415` | `unsupported_media_type` | A `POST`, `PUT` or `DELETE` without `Content-Type: application/json` |
| `421` | `untrusted_host` | The `Host` header is not accepted |
| `429` | `too_many_attempts`, `password_capacity_exhausted`, `storage_analysis_rate_limited`, `preference_write_rate_limited`, `weather_capacity_exhausted`, `strand_busy`, `terminal_capacity_exhausted`, `host_snapshot_in_progress` | A rate or concurrency limit was hit |
| `503` | `host_broker_unavailable`, `request_capacity_exhausted`, `service_unavailable`, `session_maintenance_in_progress`, `terminal_unavailable`, `weather_unavailable` | A dependency is down, or Helix is at its request limit |

Some responses include `Retry-After` in seconds:

| Code | `Retry-After` |
| --- | --- |
| `request_capacity_exhausted`, `password_capacity_exhausted`, `host_snapshot_in_progress`, `terminal_capacity_exhausted`, `session_maintenance_in_progress` | 1 |
| `host_broker_unavailable`, `terminal_unavailable`, `weather_capacity_exhausted` | 2 |
| `preference_write_rate_limited`, `storage_analysis_rate_limited`, `strand_busy`, `weather_unavailable` | 60 |
| `token_rate_limited` | 60 |
| `too_many_attempts` | 900 |

Every response carries an `X-Request-ID` header. Keep it when you report a
problem.

> [!NOTE]
> A gateway timeout comes from the gateway, not from Helix, so it is a plain
> `504` page rather than the JSON shape above. Treat it like a lost response.

## Rate and concurrency limits

| Limit | Value | Over the limit |
| --- | --- | --- |
| Requests Helix handles at once | 64 | `503 request_capacity_exhausted` |
| Token requests per token | 600 per minute | `429 too_many_attempts` |
| Automation requests in progress | 2 at a time, across all tokens | Further requests wait for a slot |
| Automation job waits in progress | 4 at a time | Further waits queue |
| Server file commands (`/servers/{id}/files`) and backup downloads | 2 at a time each | Further requests wait for a slot |
| Active chunked uploads | 2 per Helix | Broker refuses a third `upload_begin` |
| Login | Per client and per account | `429 too_many_attempts` |
| Storage analysis reads | Per user and per session each minute | `429 storage_analysis_rate_limited` |
| Preference saves | Per user and per session | `429 preference_write_rate_limited` |

Requests that wait for a slot still count against the gateway's 15-second
timeout.

## Timeouts, and why jobs exist

Three clocks apply to every request:

- **Gateway:** 15 seconds to start reading a response (`proxy_read_timeout`),
  except the terminal WebSocket.
- **Helix:** 50 seconds per API request, then `408 request_timeout`. Permanently
  deleting a removed server gets 300 seconds.
- **Busy server:** a direct call that needs a server another operation is using
  waits up to 8 seconds, then fails with a message that names the operation
  holding the server and says to try again when it finishes.

Starting, stopping, backing up, restoring, updating, installing content,
deploying, creating servers and copying between servers can take minutes. Those
calls do not run inside the request. Helix accepts them as a **job** and
returns at once:

```json
{"job_id": "0b9a6c7e-5d0f-4f7e-8f0e-2f6d2b1c9a41", "reused": false}
```

`reused: true` means the same operation was already queued or running for that
server, and you got its job instead of a second one.

A job record looks like this:

```json
{
  "id": "0b9a6c7e-5d0f-4f7e-8f0e-2f6d2b1c9a41",
  "kind": "server_deploy",
  "status": "running",
  "stage": "Waiting until the game is ready",
  "progress_percent": 75,
  "created_at_unix_ms": 1761100000000,
  "updated_at_unix_ms": 1761100042000,
  "result": null,
  "error": null
}
```

| `status` | Meaning |
| --- | --- |
| `queued` | Accepted, waiting. Jobs for the same server wait in line, and `stage` says what they wait for |
| `running` | In progress |
| `complete` | Finished. `result` holds the details |
| `failed` | Finished with an error. `error` explains it. The HTTP status of the poll is still `200` |

How to follow a job:

- Owner session: `GET /api/v1/jobs/{job_id}` (needs `games.view`). Host jobs
  also have their own status routes, such as
  `GET /api/v1/system/packages/jobs/{job_id}`.
- Token: `{"operation":"job_status","job_id":"..."}`, or
  `GET /api/v1/automation/jobs/{job_id}/wait`, which returns as soon as the job
  finishes or after up to 12 seconds. See
  [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API#jobs).

Poll about every 2 seconds. Job records are saved to disk and survive a broker
restart; a job that was running when the broker stopped is reported as
`failed` with the stage it reached. Helix keeps up to 64 job records and drops
the oldest finished ones first.

A timeout, a closed connection, or your own polling deadline never cancels a
job. Do not repeat a mutation to find out whether it worked. Check the job and
the server first.

## Retries

- Never replay a general mutation because its response was lost. Helix has no
  general idempotency keys.
- Server file uploads are the exception. `upload_file` is content-idempotent,
  and repeating `upload_begin` with the same path, size, SHA-256 and revision
  resumes the same upload. See
  [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API#uploads).
- If you retry reads, honor `Retry-After`, back off, and stop on `401` or `403`.

## Python client

The repository includes a small client that uses only the Python standard
library:
[`examples/integrations/helix_client.py`](https://github.com/Riqqqque/Helix/blob/main/examples/integrations/helix_client.py).
Copy it next to your script.

| Class | Use |
| --- | --- |
| `HelixClient(origin, allow_private_http=False, timeout=30)` | Owner session: `login`, `logout`, `discovery`, `servers`, `server`, `server_action`, `server_capabilities`, `server_files`, `iter_server_directory`, `download_file`, `upload_file`, `transfer_file`, `wait_for_job`, `job_status` |
| `HelixTokenClient(origin, token, timeout=30)` | Token: `execute(operation, **fields)`, `server_capabilities`, `server_files`, `server_action`, `job_status`, `wait_job`, `wait_for_job`, `deploy`, plus the same `download_file`, `upload_file`, `transfer_file` and `iter_server_directory` helpers |

What it does for you:

- Verifies TLS, refuses redirects, ignores proxy environment variables, and
  keeps credentials in memory. It never logs tokens or response bodies.
- Keeps requests inside `/api/v1/` on one origin. The token client can call
  only `/api/v1/automation/server` and `/api/v1/automation/jobs`.
- `upload_file` hashes the local file, uses one `upload_file` request up to
  3 MiB and resumable chunks above that, and retries only the upload steps that
  are safe to repeat.
- `download_file` checks every chunk's SHA-256 and the file revision, and
  never overwrites an existing local file.
- Does not retry general mutations.

Limits to know:

- Plain HTTP is allowed only on loopback. `HelixClient` also allows a private
  IP with `allow_private_http=True`; `HelixTokenClient` does not, so use HTTPS
  or a loopback address such as an SSH tunnel. The gateway must accept that
  address as a host (`HELIX_GATEWAY_HOST` or `HELIX_SECONDARY_GATEWAY_HOST`).
- The token client has no helper for `/api/v1/automation/jobs/{job_id}/wait`.
  Use `wait_for_job`, which polls `job_status`, or call the wait route with
  another HTTP library.

```python
import os
from helix_client import HelixTokenClient

client = HelixTokenClient("http://127.0.0.1:3100", os.environ["HELIX_SERVER_TOKEN"])
server = "helix:EXACT-UUID"
try:
    print(client.server_capabilities(server))
    client.upload_file(server, "build/MyPlugin.jar", "plugins/MyPlugin.jar")
    job = client.execute("deploy_pending_changes", instance_id=server, options={})
    print(client.wait_for_job(job["job_id"], timeout=600)["result"])
finally:
    client.close()
```

The client's tests run with
`python3 -m unittest discover -s examples/integrations -p 'test_*.py' -v`.

## Versioning

- All routes live under `/api/v1`. Within v1, new response fields can appear at
  any time; accept fields you do not know. Removing a field or changing its
  meaning needs an explicit version or deprecation decision.
- Discovery reports `helix_version` so you can tell which release you talk to.
  The [Changelog](https://github.com/Riqqqque/Helix/blob/main/CHANGELOG.md) lists
  API additions per release.
- Treat unknown job states and unknown enum values as errors, not as success.
- This is a private-LAN API. It has not been through a public compatibility
  period, and it is not a public stability promise.

## Conventions

- Servers are identified by their exact `id` from `GET /api/v1/servers`,
  including the manager prefix (`helix:` for native servers, `amp:` for
  imported AMP instances). A name, port or version never identifies a server.
- `GET /api/v1/servers` returns a JSON array.
- Timestamps end in `_unix_ms` or `UnixMs` and are UTC milliseconds.
- Sizes are bytes. Very large counters can be decimal strings.
- Protected responses use `Cache-Control: no-store`.
- Lists, logs, directory pages, search results and analyses all have explicit
  caps.

## Safety rules

- Use a token scoped to exact servers for automation, not an owner session.
- Keep secrets in environment variables, never in URLs, commits or files a
  tool reads. See
  [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens#use-a-token-from-a-script).
- A job ID means accepted, not done. Wait for `complete` and check the server.
- Back up before changing software or files, and keep the backup until the
  server has started cleanly. The deploy job does this for you; see
  [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).

## Related pages

- [Dashboard API Reference](https://github.com/Riqqqque/Helix/wiki/Dashboard-API-Reference)
- [Automation API](https://github.com/Riqqqque/Helix/wiki/Automation-API)
- [Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)
- [Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes)
- [Copying Between Servers](https://github.com/Riqqqque/Helix/wiki/Copying-Between-Servers)
- [Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands) for
  dashboard extensions
- [Server automation on the host](https://github.com/Riqqqque/Helix/blob/main/docs/SERVER-AUTOMATION.md)
  for finding server folders from a shell
