# How Helix Works

Helix is a private web control plane for one Linux host. The browser talks to
the unprivileged `helixd` service. Read-only host data comes from bounded
adapters; root-required work crosses a typed local socket to `helix-privd`.
There is no caller-supplied root command endpoint.

The native Minecraft manager creates and controls Docker-backed Paper, Purpur,
Folia, Fabric, and Vanilla instances. Console capture and game containers do not
depend on the dashboard being open. Console history is bounded and rotates; it
is persistent, not unlimited.

AMP integration is a bridge to a separate loopback AMP API. Helix may show and
invoke the AMP actions it understands, but AMP remains responsible for its own
instances and files.

The Home page is deliberately modular: widgets can be added, moved, resized,
renamed, recolored, and removed across multiple exportable layouts without
installing an extension. Hooks connects exact host services without granting a
third party a runtime inside Helix. Strands are the future third-party extension
model. Today the Strand Kit validates preview manifests only and cannot install
or execute code.

The optional terminal is not a root-broker command. A fresh dashboard-password
proof opens a one-use connection to a separate service running as one normal
Linux user. Helix audits connection lifecycle but not terminal commands/output.

The appeal is practical: one responsive interface for host health, storage,
network evidence, files, native servers, and safe host actions, while managed
games stay outside the dashboard process and player traffic path.

Helix is still a private alpha. Broad/unattended package upgrades, signed
self-update, public exposure, Forge/NeoForge/Quilt/CurseForge lifecycle support,
and broad/full-pack modpack parity are not current features. Exact selected APT
candidates have a guarded explicit path with no rollback claim. One narrow path creates a
declared-hash-verified server-safe subset from listed stable server-capable
Fabric `.mrpack` releases on Modrinth. Tailscale can sit in front of a separately
constrained private entry point. Its Hook can install and start the exact
service on eligible Debian/Ubuntu hosts, but Helix does not authenticate the
tailnet or configure the gateway trust boundary.

Read the
[full walkthrough](https://github.com/Riqqqque/Helix/blob/main/docs/HOW-HELIX-WORKS.md)
for the complete request flow and safety boundaries.
