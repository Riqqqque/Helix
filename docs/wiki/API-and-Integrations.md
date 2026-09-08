# API and integrations

Use Helix's API to connect trusted scripts and tools without guessing server
directories or exposing Docker. The API stays on the configured private entry
point; this feature does not publish your dashboard to the internet.

1. Read the [integration guide](https://github.com/Riqqqque/Helix/blob/main/docs/INTEGRATIONS.md).
2. Log in with an authorized Helix session and keep its cookie and CSRF proof together.
3. Read `/api/v1/discovery` for permissions, links and supported conventions.
4. Use the [OpenAPI contract](https://github.com/Riqqqque/Helix/blob/main/docs/openapi.json)
   or the authenticated `/api/v1/openapi.json` endpoint.
5. Start with the [Python reference client](https://github.com/Riqqqque/Helix/blob/main/examples/integrations/helix_client.py).

The contract lists every server route and method: creation, lifecycle actions,
console, files, uploads, downloads, settings, ports, marketplace, runtime,
backups, artwork and removal. Read each server's `/capabilities` response first.
The [server API guide](https://github.com/Riqqqque/Helix/blob/main/docs/SERVER-API.md)
includes the game/manager matrix, request fields, transfer examples and recovery.
Some broker responses are still open objects, not complete generated-client schemas.

Native Minecraft (including Pumpkin/custom), V Rising, Valheim and Terraria use
the same relative file commands. Uploads verify SHA-256 before publication;
replacement and deletion retain recoverable files. Upload staging can run while
online, but changes to live server files require a stopped server. Chunked backup
export is also available. Imported AMP servers keep the adapter's actual limits;
unsupported controls are not advertised as native features.

## The important safety rules

- An owner session carries owner permissions. There are **no server-scoped API
  credentials yet**. Do not give an owner login to an untrusted integration.
- Use the inventory item's `id` exactly, including `helix:` or another manager
  prefix. A display name, port or Minecraft version does not identify a server.
- A returned job ID means the operation was accepted, not that it completed.
  Poll until `complete` or `failed` and verify the server afterward.
- A timeout does not cancel an operation. Never automatically repeat a restart,
  installation or file write because its response was lost.
- Back up before updating files or software. Stop the selected server before
  replacing plugins, mods or worlds. Keep the backup until startup is verified.

For narrow third-party UI features, consider
[Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands).
For authorized host-side inventory, see
[server automation](https://github.com/Riqqqque/Helix/blob/main/docs/SERVER-AUTOMATION.md).
