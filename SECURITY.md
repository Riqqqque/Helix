# Security Policy

## Project status

Helix is pre-release founding work. It is not ready for production deployment
or public-network exposure and has not completed an independent security audit.
The detailed intended threat model is in [`docs/SECURITY.md`](docs/SECURITY.md).
That document is architecture, not a claim that its controls are implemented.

Do not use the current project to protect production servers, credentials,
worlds, saves, or backups. There are currently no supported release branches or
security-maintenance guarantees.

## Reporting a vulnerability

Do not open a public issue containing exploit details, credentials, private
server data, or a working attack against someone else's system.

Use GitHub's
[private vulnerability reporting](https://github.com/Riqqqque/Helix/security/advisories/new)
for a confidential report to the repository owner. Do not open a public issue or
discussion first. If GitHub reports that private reporting is unavailable, open
only a minimal public issue asking the owner to restore the private channel; do
not include technical details.

A useful private report includes:

- affected commit or version and installation method;
- OS, kernel, filesystem, and relevant deployment topology;
- whether the service was bound to loopback, LAN, or a public interface;
- prerequisite role/capability and required user interaction;
- reproducible steps or a minimal proof of concept;
- confidentiality, integrity, and availability impact;
- whether worlds, saves, backups, credentials, or root boundaries are involved;
- sanitized logs and crash output; and
- any known workaround that does not destroy evidence.

Remove tokens, passwords, recovery keys, private hostnames/IPs, personal data,
and world content that is not necessary to demonstrate the issue. Use synthetic
test data whenever possible.

No response or remediation SLA is promised during the founding phase. Receipt,
triage, coordinated disclosure timing, affected-version analysis, and credit
will be handled through the private reporting channel.

## Scope priorities

Especially important reports include:

- unauthenticated owner creation, authentication/session/MFA bypass, CSRF, or
  account takeover;
- authorization bypass or cross-user/cross-instance object access;
- arbitrary command execution or privilege escalation in current code; once the
  planned `helix-privd` broker exists, access outside its typed protocol;
- path traversal, symlink/hard-link/mount races, unsafe archive extraction, or
  writes outside an assigned root;
- SSRF to local, private, link-local, cloud metadata, or privileged endpoints;
- plaintext or leaked secrets in storage, logs, API/UI payloads, audit records,
  support bundles, process arguments, or backups;
- update, Sequence, Strand, runtime, game-package, or installer integrity
  bypass;
- database corruption, migration data loss, unsafe repair, failed rollback, or
  a backup labeled verified without full integrity evidence;
- a dashboard/Strand/metrics failure that stops independent games;
- unbounded memory, disk, log, queue, archive, WebSocket, or retry behavior; and
- recovery behavior that overwrites the last known-good world, state database,
  backup, or forensic copy.

## Out of scope and safety

Do not test against systems you do not own or lack explicit authorization to
assess. Do not access or retain other people's data, degrade public services,
run persistence, move laterally, exfiltrate credentials, or destroy evidence.
Use an isolated lab and the minimum action needed to prove impact.

Reports that only note an explicitly documented future capability is absent are
not vulnerabilities unless current documentation or UI claims that capability
is active. Dependency scanner output without an
exploitable Helix path is useful maintenance information but should include
reachability/context before being treated as a security defect.

## Supported versions

No version is supported for production use yet.

| Version | Security support |
| --- | --- |
| `main` / `0.1.0-alpha.1` source | Development preview only; no support guarantee |

This table changes only after a release has passed the installation,
authentication, authorization, filesystem, secret-redaction, update-integrity,
backup/restore, corruption, disk-full, power-loss, and recovery gates documented
in the project.

## Security design limits

The current `helixd` is an unprivileged, loopback-only web daemon. The narrow
root broker described in the architecture is future work and does not exist in
this repository. Even after that broker is implemented, unrestricted root on
the running host can inspect process memory, replace binaries and keys, forge
local records, and alter data before backup. Helix cannot make a dishonest
guarantee against a compromised kernel, firmware, or storage device that lies
about durable writes.

Backups are not verified because an archive opens. Encryption is not complete
unless the protected data, key placement, recovery path, algorithms, and threat
boundary are documented and tested. A package signature proves origin/integrity,
not safety. Native extensions with root or broad host access are trusted code,
not a sandbox.

## Disclosure and release handling

Once supported releases exist, a security fix must include:

- affected and fixed versions;
- severity and prerequisites without unnecessary exploit enablement;
- migration, restart, credential-rotation, and recovery steps;
- checksums/signatures and verified release provenance;
- tests that fail before and pass after the fix;
- an assessment of exposed secrets/data and whether rotation or restore is
  required; and
- clear notice when validation is incomplete.

Security fixes are not silently weakened to preserve compatibility. Any
temporary mitigation and residual risk must be explicit.
