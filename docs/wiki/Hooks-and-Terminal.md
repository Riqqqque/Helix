# Hooks and Terminal

## Hooks

Hooks brings services into Helix without pretending Helix owns their data or
complete API. The built-in catalog currently knows about:

- Plex Media Server;
- AMP;
- Tailscale;
- Pterodactyl Wings; and
- Jellyfin.

The protected broker configuration decides which exact systemd units are
controllable. Installed services report running and start-after-boot state.
Supported buttons issue only start, stop, restart, enable, or disable for that
exact unit and verify the result. AMP uses its separate loopback API adapter.

When a service is absent, Helix provides concise setup steps and links to the
official instructions. It does not run a remote shell script, add a package
repository, invent an account credential, or silently authenticate a tailnet.
Once the supported service exists, **Check connections** detects it.

Open-service buttons use the local Helix hostname with the known panel port.
Deep library, account, network, or application-specific settings remain in the
upstream interface unless a future typed adapter can validate them safely.

## Host terminal

Terminal is optional and has a stronger warning because it is a real Linux PTY.
It runs as one configured non-root Linux user, not as `helix-privd`. Normal host
policy still applies; `sudo` may ask for that Linux user's password and may grant
root only if the account is already authorized.

Each new connection requires the current Helix dashboard password. The proof
creates a random 30-second, single-use, session-bound HttpOnly ticket. The
browser then opens one exact WebSocket protocol. The host service uses a group
separate from the privileged broker and Linux peer credentials restrict socket
clients to the pinned dashboard UID.

Helix audits password rejection and session opened/closed/failed lifecycle. It
does not store commands, keystrokes, output, or environment values. The browser
keeps 10,000 lines of local scrollback only for that page. Disconnecting or
closing the page kills the PTY; use `tmux` or a system service for work that must
survive a browser disconnect.

If the page says unavailable, verify the optional `helix-terminald@USER` service,
its separate socket group, `/run/helix/terminal/terminal.sock`, the dashboard's
pinned UID/GID mapping, and the exact gateway WebSocket route. Do not solve it by
adding the terminal user to the privileged broker group.
