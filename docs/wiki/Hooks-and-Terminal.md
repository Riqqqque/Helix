# Hooks and Terminal

**Hooks** connect Helix to services that already run on your host, such as
Plex, Jellyfin, Tailscale, Docker, or AMP. Helix shows their state and offers
a few exact controls, but each service keeps its own data and settings.

**Terminal** is an optional real Linux shell in the browser. It runs as one
normal Linux user, never as root and never through the broker.

## Hooks

### What a Hook is

A Hook is a card on the **Hooks** page for one known service. Each card shows:

- whether the service is installed, running, stopped, or needs attention;
- whether it starts after boot;
- memory use and one short CPU sample, when systemd reports them; and
- a link to the service's own web interface, when it has one.

Helix does not import the service's data or copy its settings. Library,
account, network, and app-specific settings stay in the service's own
interface.

### Available Hooks

| Hook | Connects through | Controls | Opens |
| --- | --- | --- | --- |
| Plex | systemd unit `plexmediaserver.service` | Start, Stop, Restart, start after boot | Plex Web on port 32400 |
| Jellyfin | systemd unit `jellyfin.service` | Same, plus a one-click installer | Jellyfin Web on port 8096 |
| Tailscale | systemd unit `tailscaled.service` | Same, plus a one-click installer | The Tailscale admin console |
| Pterodactyl Wings | systemd unit `wings.service` | Same, plus a guided setup check | Nothing; Wings has no web UI of its own |
| AMP | AMP's loopback API, configured in the broker | The AMP actions Helix understands | The AMP panel, if a panel port is set |
| Docker | Docker Engine on this host | Start, Stop, Restart for containers | Portainer, if found |
| Portainer | A Portainer container Helix finds | None | The Portainer UI on 9443, then 9000 |

The Docker card lists every container on the host, including stacks Portainer
also manages. Live CPU and memory come from Docker when you open its panel.
Start, stop, and restart there refuse the Helix dashboard and gateway
containers. Native game servers are controlled from the Servers page instead.

Portainer port 8000 is skipped because it is an Edge connection port, not a web
console.

### Which units Helix may control

The broker only controls units listed in `host_control.hook_services` in
`/etc/helix/privd.json`. If you leave it out, the defaults are:

```json
{
  "host_control": {
    "hook_services": [
      { "id": "plex", "unit": "plexmediaserver.service" },
      { "id": "tailscale", "unit": "tailscaled.service" },
      { "id": "pterodactyl", "unit": "wings.service" },
      { "id": "jellyfin", "unit": "jellyfin.service" }
    ]
  }
}
```

Up to 32 entries are allowed. IDs and unit names must be unique. An entry with
an ID Helix does not recognize shows as a generic **Custom** card.

Each control sends only `start`, `stop`, `restart`, `enable`, or `disable` for
that exact unit, then checks the result. The browser cannot name a unit, a
package, or a command.

Controls need the `system.settings.write` capability.

### Installing a missing service

When a service is not installed, its card offers setup instead of controls.

1. **Preflight.** Helix runs a read-only check of the host: Linux release,
   CPU architecture, and required tools (`curl`, `systemctl`, `timeout`, and
   for APT installs `apt-get` and `dpkg`). It lists every file it will write.
2. **Review.** The plan shows each check as pass or block, the planned writes,
   and the steps that stay with you.
3. **Install.** If nothing blocks, you start the install job. It shows
   progress and can be followed after a reload.

| Hook | Mode | Requirements |
| --- | --- | --- |
| Tailscale | One click | Debian, Ubuntu, or a derivative that reports a Debian or Ubuntu codename. `amd64` or `arm64`. |
| Jellyfin | One click | Same releases. `amd64`, `arm64`, or `armhf`. |
| Pterodactyl Wings | Guided | Linux on `amd64` or `arm64` with Docker and systemd. Helix checks prerequisites only. |

Other distributions get a clear "blocked" result instead of a fake install.

What a one-click install does:

1. Downloads the publisher's signing key and repository definition, only from
   `https://pkgs.tailscale.com/` or `https://repo.jellyfin.org/`.
2. Validates the repository definition.
3. Writes the key and APT source:

   | Hook | Files |
   | --- | --- |
   | Tailscale | `/usr/share/keyrings/tailscale-archive-keyring.gpg`, `/etc/apt/sources.list.d/tailscale.list` |
   | Jellyfin | `/etc/apt/keyrings/jellyfin.gpg`, `/etc/apt/sources.list.d/jellyfin.sources` |

   Downloads are staged in `/run/helix/hook-installs`.
4. Installs exactly the `tailscale` or `jellyfin` package.
5. Enables and starts `tailscaled.service` or `jellyfin.service` and checks
   that it is active.

Safety rules:

- Helix never runs a downloaded script.
- Missing target folders are created as root with a tight mode. Group- or
  world-writable folders are tightened to `0755` or `0700` before a file lands
  there. A symbolic link or a folder not owned by root blocks the install, and
  the error names the path.
- If repository setup fails before the package install, the previous
  repository files are restored. After APT has changed the host, there is no
  automatic rollback.
- Hook installs and system package updates share one lock, so they never run
  at the same time.

### After the install

- **Tailscale.** Run `tailscale up` in the Terminal and approve the one-time
  link in the tailnet you intend. Helix never stores Tailscale credentials or
  picks tailnet policy.
- **Jellyfin.** Open Jellyfin Web on port 8096 and finish its first-run owner
  and library setup.
- **Pterodactyl Wings.** Create the node in your Pterodactyl Panel, copy its
  generated `config.yml` into `/etc/pterodactyl`, then install Wings from the
  official instructions. The panel has to generate that file, so Helix cannot
  honestly make this one click.

## Terminal

### What you get

The Terminal page opens an interactive login bash on the host, as the Linux
user you chose at install. It behaves like an SSH session:

- your profile, `~`, and Tab completion work;
- `sudo` follows the host's normal rules and may ask for that user's password;
- programs like `htop` and `vim` work, including mouse reporting; and
- `http://` and `https://` links in output open in a new tab.

The **Quick checks** bar types these read-only commands into the same shell:

| Button | Command |
| --- | --- |
| Files | `ls -lah --color=auto` |
| Drives | `df -h --output=source,size,used,avail,pcent,target` |
| Failed services | `systemctl --failed --no-pager` |

### Keys and controls

| Action | Keys or button |
| --- | --- |
| Copy | Ctrl+Shift+C, or Cmd+C on macOS, or **Copy** |
| Paste | Ctrl+Shift+V, or Cmd+V, or **Paste** |
| Interrupt | Ctrl+C (sent to the shell as usual) |
| Find in scrollback | Ctrl+F; F3 and Shift+F3 for next and previous |
| Text size | Ctrl and `+`, `-`, `0`; Ctrl+mouse wheel; **A−** and **A+** |
| Clear the screen | **Clear view** |
| End the session | **Disconnect** |

The browser keeps 10,000 lines of scrollback for this page only. Text size is
remembered in this browser.

### How a connection is opened

1. You enter your Helix dashboard password. Every new connection needs it.
2. Helix creates a single-use ticket bound to your session. It lasts 30
   seconds and is sent as an HttpOnly cookie. At most 16 tickets can be
   pending at once.
3. The browser opens a WebSocket to `/api/v1/terminal/connect` with the exact
   `helix-terminal-v1` protocol. The gateway allows up to 8 hours on that
   route; other routes time out after 15 seconds.
4. `helixd` connects to `/run/helix/terminal/terminal.sock`. That socket
   belongs to the `helix-terminal` group, which is separate from the broker's
   `helix-broker` group.
5. `helix-terminald` checks the caller's Linux user ID with peer credentials
   and accepts only the dashboard's ID (10001).
6. It starts `bash -i -l` with a cleared environment (`HOME`, `TERM`, and a
   UTF-8 locale are set fresh).

Closing the page or pressing **Disconnect** ends the shell. Use `tmux`, or a
systemd service, for work that must keep running after you leave.

### Limits

| Limit | Value |
| --- | --- |
| Sessions at once | 8 |
| Terminal size | 20 to 400 columns, 5 to 200 rows |
| Frame size | 64 KiB |
| Single input message | 32 KiB |

The Machines page also opens SSH sessions to other computers inside this same
PTY service. That is the only extra program the service will launch.

### What Helix records

Helix audits a rejected password and each session being opened, closed, or
failing. It does not store commands, keystrokes, output, or environment
values.

### Enabling it

The installer enables it when you pass a user:

```bash
sudo ./scripts/install.sh --terminal-user <linux-user>
```

The user must exist, must not be `root`, and must not be in the `helix-broker`
group. To set it up by hand:

```bash
sudo install -m 0755 target/release/helix-terminald /usr/local/libexec/helix-terminald
sudo install -m 0644 deploy/helix-tmpfiles.conf /etc/tmpfiles.d/helix.conf
sudo install -m 0644 deploy/helix-terminald@.service /etc/systemd/system/helix-terminald@.service
sudo systemd-tmpfiles --create /etc/tmpfiles.d/helix.conf
sudo systemctl daemon-reload
sudo systemctl enable --now helix-terminald@<linux-user>.service
```

The unit runs as `<linux-user>` in group `helix-terminal` and pins the
dashboard peer ID to 10001. Using the page needs the `terminal.open`
capability.

### If the page says unavailable

Check, in order:

1. `systemctl status helix-terminald@<linux-user>` is active.
2. `/run/helix/terminal/terminal.sock` exists, owned by that user and group
   `helix-terminal`, mode `0660`, inside a `0770` folder.
3. The dashboard container has the `helix-terminal` group ID
   (`HELIX_TERMINAL_GID` in `.env`) and still runs as 10001.
4. The gateway's `HELIX_BROWSER_ORIGIN` matches the address in your browser.

Do not fix it by adding the terminal user to `helix-broker`. That would give a
normal shell access to the root broker.

## Related pages

- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery)
- [Network, Host, and Updates](https://github.com/Riqqqque/Helix/wiki/Network-Host-and-Updates)
- [Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration)
- [Glossary](https://github.com/Riqqqque/Helix/wiki/Glossary)
