# Hooks and Terminal

## Hooks

Hooks brings services into Helix without pretending Helix owns their data or
complete API. The built-in catalog currently knows about:

- Plex Media Server;
- AMP;
- Tailscale;
- Pterodactyl Wings;
- Jellyfin; and
- Docker, including every container on the host and Open Portainer when that
  container is published.

The protected broker configuration decides which exact systemd units are
controllable. Installed services report running, start-after-boot state, and
cgroup memory when systemd exposes it. Supported buttons issue only start, stop,
restart, enable, or disable for that exact unit and verify the result. AMP uses
its separate loopback API adapter. Docker start/stop/restart uses the typed
container-action route and refuses Helix dashboard/gateway names.

When a service is absent, Helix runs a read-only host preflight first. On a
supported Debian or Ubuntu release and architecture, Tailscale and Jellyfin
have one-click installers. Each installer accepts only one built-in hook ID,
downloads repository material from the publisher's exact HTTPS host, validates
the repository definition, adds the exact signed APT source, installs the exact
allowlisted package, enables the exact systemd service, and verifies that it is
active. Hook installs and System Updates share one package-operation lock.

Helix never runs a downloaded root script and the browser cannot supply a
package, repository, unit, executable, or shell command. If repository setup or
APT fails before package installation, prior repository files are restored.
There is no general package rollback claim after APT has changed the host.

Tailscale account login is deliberately separate. After the package and
`tailscaled.service` are ready, the owner runs `tailscale up` in Terminal and
approves the one-time link in the intended tailnet. Helix neither stores those
credentials nor chooses tailnet policy. Jellyfin likewise leaves its first-run
owner and media-library wizard inside Jellyfin Web.

Pterodactyl Wings is guided instead of falsely labeled one-click. Helix checks
the supported Linux release, architecture, Docker, systemd, and command
prerequisites, then identifies the remaining owner steps. The node must first
exist in a Pterodactyl Panel because that panel generates the node-specific
`config.yml` and credentials.

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

The quick-check buttons send ordinary read-only commands into the same PTY.
Output remains terminal-native so ANSI colors, wrapping, prompts, columns, and
interactive programs behave like an SSH terminal rather than a reformatted log
viewer.

If the page says unavailable, verify the optional `helix-terminald@USER` service,
its separate socket group, `/run/helix/terminal/terminal.sock`, the dashboard's
pinned UID/GID mapping, and the exact gateway WebSocket route. Do not solve it by
adding the terminal user to the privileged broker group.
