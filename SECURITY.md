# Security Policy

## Project status

Helix is a private alpha. It has real authentication, capability checks, a
typed root broker, configured-root file operations, native game management, and
a constrained private gateway, but it has not completed an independent security
review or a public-network deployment review.

Do not expose the current build directly to the public internet or use Helix as
the only protection or copy for credentials, worlds, saves, or backups. There
are no supported release branches or security-maintenance guarantees yet.

The detailed current boundaries and open gates are in
[`docs/SECURITY.md`](docs/SECURITY.md) and
[`PROGRESS.md`](PROGRESS.md).

## Reporting a vulnerability

Do not open a public issue containing exploit details, credentials, private
server data, or a working attack against someone else's system.

Use GitHub's
[private vulnerability reporting](https://github.com/Riqqqque/Helix/security/advisories/new)
for a confidential report. If that channel is unavailable, open only a minimal
public issue asking for it to be restored; do not include technical details.

A useful private report includes:

- affected commit/version and installation method;
- OS, kernel, architecture, filesystem, and deployment topology;
- whether the service was loopback, private-LAN, private-VPN, or publicly
  reachable;
- required role/capability, prerequisites, and user interaction;
- reproducible steps or a minimal proof of concept;
- confidentiality, integrity, availability, and recovery impact;
- whether root, credentials, worlds, backups, AMP, Docker, UFW, or system power
  are involved;
- the smallest sanitized logs needed to understand the issue; and
- a non-destructive workaround, if known.

Remove passwords, tokens, cookies, CSRF proofs, private addresses/hostnames,
storage paths, personal data, world content, and unrelated logs. Use synthetic
test data whenever possible.

No response or remediation SLA is promised during the alpha. Receipt, triage,
coordinated disclosure, affected-version analysis, and credit are handled
through the private channel.

## Priority areas

Especially useful reports include:

- first-owner races, authentication/session bypass, CSRF/Origin confusion,
  account takeover, or capability/IDOR failures;
- arbitrary command execution, privilege escalation, broker protocol escape,
  socket impersonation, or operations outside the typed request set;
- path traversal, symlink/hard-link/mount races, unsafe archive handling, or
  access outside configured storage/native/backup roots;
- unsafe Docker identity matching, cross-instance actions, or concurrency that
  corrupts a world or backup;
- AMP credential leakage, non-loopback exposure, identity confusion, or an
  ambiguous response treated as success;
- UFW changes to unowned rules/defaults, router/open-port overclaims, unsafe
  reboot scheduling, or start-on-boot changes to unrelated containers;
- SSRF, redirect/DNS/proxy abuse, malicious Minecraft/runtime/Modrinth metadata,
  or artifact integrity bypass;
- secrets in storage, logs, errors, API/UI payloads, audit, process arguments,
  backups, or support data;
- migration data loss, unsafe repair, failed rollback, recoverable trash escape,
  or a backup labeled verified without matching evidence;
- unbounded memory, CPU, disk, logs, console history, queues, retries, responses,
  or storage analysis; and
- a dashboard/broker failure that stops an independent game container.

Broad/unattended Package Apply, Helix self-update, public exposure, third-party
Strand execution, and unsupported Minecraft loader/modpack paths are absent by
design. Exact selected APT candidates do have a guarded explicit update path;
it makes no rollback claim and never reboots automatically. The one
modpack path is limited to declared-hash-verified server-safe subsets from
listed stable server-capable Fabric `.mrpack` releases; non-Fabric loaders and
full-pack parity remain unavailable. Reporting only that an explicitly
unavailable feature is absent is not a vulnerability. A disabled control that
mutates anyway, reports false success, or bypasses its boundary is in scope.

## Current security boundaries

- Source development defaults to loopback. The checked gateway can be
  constrained to one private address/Host/Origin/client CIDR. Public exposure is
  unsupported.
- `helixd` is unprivileged. `helix-privd` is a separate root service with a
  closed, length-bounded request protocol and no general shell RPC.
- The broker socket currently relies on filesystem ownership/mode and a
  dedicated group; independent peer-credential checking remains open.
- Native Minecraft instances run in Docker through exact broker-managed
  identities. AMP stays a separate loopback integration.
- Fabric `.mrpack` creation re-resolves opaque Modrinth IDs, verifies declared
  archive/file hashes and strict extraction limits, and activates only a fresh
  server-safe subset; it is not a general modpack installer.
- File, native-state, backup, and network-rule operations are restricted to
  configured roots and opaque identities.
- Host reboot requires exact hostname confirmation, acknowledgement, workload
  preflight, a bounded delay, and a cancellable systemd timer.
- Firewall writes affect exact Helix-owned UFW rules only. An inactive UFW can
  be enabled only through a separate confirmed SSH-safety flow; Helix never
  resets or changes defaults and cannot prove outside reachability.
- Exact selected APT candidates can be applied after revalidation, no-removal
  simulation, disk/conffile checks, explicit disruption acknowledgement, and
  final-version verification. There is no signed Helix self-update route.
- The optional host terminal is a separate non-root Linux service. A 30-second
  one-use ticket requires a fresh Helix password proof, and the socket checks a
  distinct group plus the dashboard process UID through `SO_PEERCRED`. Helix
  records lifecycle authorization events, not terminal input or output.
- Tailscale may provide an already configured private route; Helix does not
  install or manage it.
- The Strand Kit validates preview metadata only. No third-party extension
  runtime or sandbox is active.

Unrestricted root or a compromised kernel/firmware can inspect memory, replace
binaries and keys, forge local records, alter containers/firewall/timers, and
modify data before backup. Helix cannot provide a guarantee against that trust
boundary.

## Testing safety

Test only systems and data you own or have explicit authorization to assess.
Use an isolated lab and the minimum action needed to prove impact. Do not access
or retain other people's data, degrade public services, establish persistence,
move laterally, exfiltrate credentials, reboot a live shared host, or destroy
evidence.

Dependency-scanner output is useful, but include reachability and an exploitable
Helix path before treating it as a security defect.

## Supported versions

No version is supported for production use.

| Version | Security support |
| --- | --- |
| `main` / `0.1.0-alpha.1` source | Private development preview; no support guarantee |

This changes only after the documented installation, authentication,
authorization, broker, filesystem, secret-redaction, update-integrity,
backup/restore, corruption, disk-full, power-loss, and recovery gates pass.

## Disclosure and release handling

Once supported releases exist, a security fix must identify affected/fixed
versions, prerequisites, migration/restart/rotation/recovery steps,
checksums/signatures, regression tests, exposed data or credentials, and any
remaining validation gap. Security controls are not silently weakened to keep
compatibility.
