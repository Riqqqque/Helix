# Security and Recovery

## Current boundary

Helix 1.0 is a private-LAN release. Source previews default to loopback. The Compose
example can expose an exact private-LAN address through a constrained gateway,
but public internet exposure has not passed review. A private Tailscale route
may use a separately constrained entry point. The built-in Hook can install and
start the exact service on eligible Debian/Ubuntu hosts, but the owner still
authenticates the tailnet and configures the gateway trust boundary. Install
errors name the directory Helix refused, instead of a generic ownership line.

The current owner flow uses:

- a random, single-use, short-lived local setup token;
- Argon2id password hashing and revocable sessions (30 minutes idle and eight
  hours by default; Settings can turn that expiry off);
- `HttpOnly`, host-only, `SameSite=Strict` cookies;
- a session-bound CSRF proof required for protected reads and writes and kept
  only for this exact browser origin so a refresh can revalidate the session;
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

Host reboot requires power permission, a confirmation dialog, and workload
preflight. The request includes the verified hostname and acknowledgement.
Immediate reboots have no countdown and cannot be cancelled; delayed API
requests use a cancellable systemd timer. Firewall writes affect only
exact Helix-owned UFW rules. A separate flow can enable inactive UFW after
preserving a verified listening SSH port; it never resets or changes defaults.
Exact selected APT candidates have a guarded update path with no rollback claim.
Helix self-update can apply a SHA-256-pinned GitHub source archive to the
dashboard, gateway, and broker, then restore those if health-check fails.
Docker cleanup is a separate fixed broker operation. It does not expose a shell,
path, container selector, volume flag, or all-images flag. Scheduled and manual
runs preserve containers and volumes, use a minimum age, and stop at the first
failed cleanup step with a durable result record.

## Security center

The **Security** page is host-first. Cards cover firewall, SSH, kernel,
Fail2ban, NTP, AppArmor, and Docker live-restore, plus short recommendations
for a private game box. Helix-only items (CSRF, LAN bind, start-after-boot,
Minecraft auto-forward, typed broker) live under the Helix filter.

Writable switches still require typing an exact confirmation phrase:

- start Helix after host boot;
- forward Minecraft ports when creating a server;
- enable or disable `unattended-upgrades` when that unit is already installed;
- enable or disable Fail2ban when that unit is already installed;
- enable or disable systemd-timesyncd when that unit is already installed.

Helix will not disable UFW from this page, rewrite sshd, or offer a root shell.
Sysctl values are observed, not rewritten. CSRF pairing, the private LAN bind,
AppArmor, SSH directives, ASLR, and Docker live-restore are explained, not
casual toggles. The page reads those facts directly (UFW status, the two Helix
container restart policies, the Minecraft create-forward flag) instead of
waiting on the full Host or Network inventories.

## Data and recovery

Critical state and replaceable metrics use separate durability domains. Native
console archives and backups are bounded on disk; backup deletion moves exact
known artifacts into recoverable trash. File deletion also uses configured
trash rather than claiming an irreversible delete.

### Server API tokens at rest

Helix authenticates server API tokens with a one-way fingerprint. So the owner
can view a token again, it also keeps an encrypted copy (XChaCha20-Poly1305,
envelope-encrypted under a master key). That key lives in `secrets/` in the data
folder, outside the `state/` database folder, so a copied state database or a
`helixctl backup-state` snapshot does not reveal any token; someone with the
whole data volume could. Viewing a token needs the owner's current password and
is audited. Revoking or rotating a token deletes its stored copy. Changing the
owner password invalidates every token.

### Game server safety

Game servers run in containers under their own numeric users. The broker never
follows links that a server places in its own folder when it reads or writes
there, and a server's Files tab cannot leave that server's folder. Copying a
setup between servers reads the source inside a locked-down container running
as the source's user, with no network, so links in a server folder cannot reach
host files.

Server software updates, modpack updates, restores, and copies between servers
all take a verified full backup first and restore it automatically if the server
does not start afterwards.

### Broker startup

The broker checks every configured storage root at startup. A root that no
longer exists — for example after uninstalling AMP — is skipped with a warning
in the broker's journal instead of stopping the broker. A root that exists but
is unsafe, such as a symbolic link, still stops it. If the dashboard shows the
broker as unavailable, check `journalctl -u helix-privd` on the host.

### State snapshots

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
