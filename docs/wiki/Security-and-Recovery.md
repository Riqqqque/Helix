# Security and Recovery

## Current boundary

Helix is a private alpha. Source previews default to loopback. The Compose
example can expose an exact private-LAN address through a constrained gateway,
but public internet exposure has not passed review. A private Tailscale route
may use a separately constrained entry point. The built-in Hook can install and
start the exact service on eligible Debian/Ubuntu hosts, but the owner still
authenticates the tailnet and configures the gateway trust boundary.

The current owner flow uses:

- a random, single-use, short-lived local setup token;
- Argon2id password hashing and revocable bounded sessions;
- `HttpOnly`, host-only, `SameSite=Strict` cookies;
- a session-bound CSRF proof required for protected reads and writes;
- exact capabilities enforced by the API; and
- bounded authentication work and generic failures without secret details.

`helixd` remains unprivileged. Root-required operations go to `helix-privd`
over a group-protected Unix socket as a closed, length-bounded request enum.
The broker has no general shell endpoint, but its socket boundary still relies
on filesystem/group isolation rather than an independent peer-credential check.
That and full clean-host sandbox testing remain release gates.

The optional terminal is separate from the root broker. It runs as one normal
Linux user and requires a fresh Helix password for each 30-second one-use
ticket. Its distinct socket group also checks the dashboard UID through Linux
`SO_PEERCRED`. Helix records only authorization/lifecycle events, not commands
or output. Normal `sudo` policy still applies inside that shell.

Host reboot requires exact hostname confirmation, acknowledgement, workload
preflight, a delay, and a cancellable systemd timer. Firewall writes affect only
exact Helix-owned UFW rules. A separate flow can enable inactive UFW after
preserving a verified listening SSH port; it never resets or changes defaults.
Exact selected APT candidates have a guarded update path with no rollback claim.
Helix self-update remains unavailable.

## Security center

The **Security** page is a first-class dashboard section. Each card explains
what the control does, why Helix recommends it, and what changes if you turn
it off. Recommended items are the defaults for a private LAN dashboard.

Writable switches still require typing an exact confirmation phrase:

- start Helix after host boot;
- forward Minecraft ports when creating a server;
- enable or disable `unattended-upgrades` when that unit is already installed.

Helix will not disable UFW from this page, rewrite sshd, or offer a root shell.
CSRF pairing, the private LAN bind, AppArmor, SSH PermitRootLogin, ASLR, and
Docker live-restore are observed and explained, not casual toggles.

## Data and recovery

Critical state and replaceable metrics use separate durability domains. Native
console archives and backups are bounded on disk; backup deletion moves exact
known artifacts into recoverable trash. File deletion also uses configured
trash rather than claiming an irreversible delete.

`helixctl backup-state` creates a verified critical-state snapshot. Native
backup creation and restore exist, but complete clean-host, interrupted-write,
disk-full, and off-host recovery drills are not finished. Helix must not be the
only copy of valuable state or worlds.

Read the
[full security model](https://github.com/Riqqqque/Helix/blob/main/docs/SECURITY.md)
and
[recovery contract](https://github.com/Riqqqque/Helix/blob/main/docs/RECOVERY.md).

Report vulnerabilities through
[GitHub private vulnerability reporting](https://github.com/Riqqqque/Helix/security/advisories/new),
never a public issue.
