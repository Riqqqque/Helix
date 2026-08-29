# How Helix Works

Helix is a private web control plane for one Linux host. The browser talks to
the unprivileged `helixd` service. Read-only host data comes from bounded
adapters; root-required work crosses a typed local socket to `helix-privd`.
There is no caller-supplied root command endpoint.

The native Minecraft manager creates and controls Docker-backed Paper, Purpur,
Folia, Leaves, Fabric, Forge, NeoForge, Quilt, Pufferfish, and Vanilla instances.
Marketplace install copies a compatible JAR into plugins/ or mods/ and leaves
restart to you. Console capture and game containers do not depend on the dashboard
being open. Minecraft RCON is loopback-only (127.0.0.1 on this host, not a player
port). Console history is bounded and rotates; it is persistent, not unlimited.
Native Stop waits for a clean shutdown; Kill is a confirmed SIGKILL for when that
stop hangs. AMP instances do not get Kill. Native servers can opt into Docker
start-on-boot. Owner setup can skip the Servers page and Settings can restore it
later.

V Rising installs the official dedicated server into an isolated Helix
container; the host never receives Wine packages.

AMP integration is a bridge to a separate loopback AMP API. Helix may show and
invoke the AMP actions it understands, but AMP remains responsible for its own
instances and files. Helix will not steal a live AMP game port or rewrite AMP
configs; leftover AMP UPnP mappings can be removed only after an exact typed
confirmation.

The Home page is deliberately modular: widgets can be added, moved, resized,
renamed, recolored, and removed across multiple exportable layouts without
installing an extension. Globe is a built-in map of this host and country-level
connections; it stays off the sidebar until you add it. Hooks connects exact host services without granting a
third party a runtime inside Helix. Strands are installable UI-only packages:
drop a `.strand.zip` or paste an https zip URL, review the host calls, and
Enable. They cannot call helix-privd or open a shell. Portable Wasm is still
not a runtime.

The optional terminal is not a root-broker command. A fresh dashboard-password
proof opens a one-use connection to a separate service running as one normal
Linux user. Helix audits connection lifecycle but not terminal commands/output.

The appeal is practical: one responsive interface for host health, storage,
network evidence, files, native servers, and safe host actions, while managed
games stay outside the dashboard process and player traffic path.

Helix is still a private alpha. Broad/unattended package upgrades, signed
self-update, and public internet exposure are not current features. Native
Minecraft create includes Paper, Purpur, Folia, Leaves, Fabric, Forge, NeoForge,
Quilt, Pufferfish, and Vanilla. Modpack create can use Modrinth `.mrpack` or
public CurseForge `manifest.json` packs as a server-safe subset, not a full
client copy. Valheim and Terraria have native create with file-drop mods.
Exact selected APT candidates have a guarded explicit path with no rollback
claim. Tailscale can sit in front of a separately constrained private entry
point. Its Hook can install and start the exact service on eligible
Debian/Ubuntu hosts, but Helix does not authenticate the tailnet or configure
the gateway trust boundary.

Read the
[full walkthrough](https://github.com/Riqqqque/Helix/blob/main/docs/HOW-HELIX-WORKS.md)
for the complete request flow and safety boundaries.
