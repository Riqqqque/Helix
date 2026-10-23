# Network, Host, and Updates

This page covers the host side of Helix: the Host page, the Network page and
UFW, router forwarding for game servers, Linux package updates, reboots, Docker
cleanup, and updating Helix itself. Every one of these actions goes through the
host broker (`helix-privd`) as a fixed, typed request. None of them gives the
browser a shell.

> [!CAUTION]
> Helix is a private-LAN dashboard. Forward game ports if you want outside
> players, but never forward the dashboard port.

## Host overview

**Host** shows live CPU, memory, disk, network, services, processes, and Docker
containers. **Linux updates** sits at the top, above services and processes.
Service and process tables are paged rather than one long list. Hover or focus
the information icons for definitions.

**Helix footprint** counts the dashboard, gateway, and broker only. Game servers
are shown separately, so their memory is never reported as dashboard cost.

Host lists every Docker container on the machine, with CPU and memory when
Docker reports them. Empty published ports are normal. Start, stop, and restart
need the exact container name. The Helix dashboard, gateway, and native game
containers are protected here; restart a Helix game from **Servers**.

## Network

### What the Network page shows

The Network page keeps five facts apart, because each one proves only itself:

| Fact | What it proves | What it does not prove |
| --- | --- | --- |
| Local listeners | A process is listening on this host | That Docker published it |
| Docker publications | A container port is published on the host | That UFW or the router allows it |
| UFW state and rules | The host firewall allows the port | That a remote player can connect |
| Router mapping (UPnP) | The router has a matching forward | That your ISP lets traffic in |
| Outside reachability | Always shown as **unverified** | Helix cannot test from the internet |

Each server's Overview shows a LAN address, a Tailscale address when one is
detected, and the public IP plus game port when a WAN address is available.

### Addresses

Helix reads the server's private addresses, a Tailscale address when present,
and the router's WAN address when the router reports it. A WAN address that is
private, reserved, or in the carrier-grade NAT range (`100.64.0.0/10`) is shown
as blocked, not public. Helix cannot bypass CGNAT.

### UFW rules

Rule changes are available only when UFW is installed, active, and verified.

- Named TCP or UDP rules can cover one port or a range of up to 1,024 ports.
- Helix marks its own rules with an exact opaque comment and keeps a private
  record of each one. It changes only rules it owns.
- It never deletes unknown rules, resets UFW, or changes UFW's default
  policies.
- Every change is verified afterwards. A deleted Helix rule can be restored with
  **Undo** for 15 minutes.
- Changes are spaced at least 2 seconds apart.

Each server's **Prepare host firewall** adds that game's exact rules when UFW is
already active. **Remove Helix host rules** removes only the rules that server
owns.

### Enabling UFW

If UFW is installed but inactive, **Enable UFW safely** asks for your current
SSH TCP port and the exact phrase `ENABLE UFW`. Helix first checks that the port
is really listening, adds an allow rule for it, enables UFW, and verifies both.
If verification fails, it tries to put UFW back to inactive.

This is a host-wide firewall change. Try it on a machine you can reach another
way first, such as a local keyboard.

The dashboard port also needs a rule when UFW is active. The installer offers
it, or you can add it yourself:

```bash
sudo ufw allow from 192.0.2.0/24 to any port 3100 proto tcp comment "Helix dashboard"
```

## Router port forwarding

Players outside your network need a port forward on your router to the game
port on the server's LAN address. Forward the **game** ports only.

| Game | Forward |
| --- | --- |
| Minecraft Java, Terraria | TCP game port |
| Hytale | UDP game port |
| V Rising | UDP game port and query port |
| Valheim | UDP game port and the next one (2456 to 2457 by default) |

Other games list their ports on the server's **Ports** card, with the real
protocol for each. The Minecraft creation dialog shows the exact port and
destination.

### Public access through UPnP

For native servers, Helix can ask a UPnP router on the local network to add the
forward. It:

- talks only to a router that answers on the local link, over plain HTTP to a
  private address, with no redirects;
- checks for an existing TCP or UDP mapping on that port first;
- verifies the exact address, port, and description after adding it, and
  records that the mapping is Helix's;
- adds a UFW rule only if UFW is already active; and
- removes only a mapping it owns that still matches what it recorded.

If AMP already holds the port, Helix names the AMP instance when it can and
tells you to change the port in AMP. Helix does not edit AMP files. If the AMP
instance is gone and only an old router mapping whose description starts with
`AMP` remains, a separate confirmed action can remove that mapping.

A confirmed mapping is reachable from the whole internet. Scanners find common
game ports on their own, so strangers may connect even if you never shared the
address. For Minecraft, turn on **Whitelist**. Turn public access off to keep a
server LAN- or Tailscale-only.

Helix cannot configure a router without UPnP, bypass CGNAT, or prove that a
port is reachable. Test from another network, such as a phone on mobile data.

## Globe

The Globe page and its Home widget map established public TCP connections to
country centroids. Game-port pins are connections whose local port is a
published game port, including pings and join attempts, not only players who
logged in. Outbound pins are the rest.

Helix does not geolocate private, loopback, link-local, multicast, or CGNAT
addresses, and it does not send remote IP addresses to the browser. If the WAN
address is missing or not public, destination countries still plot without a
host pin. The Home widget fills its card, so poles or the date line may be
cropped; the Globe page shows the full map.

## Linux updates

Opening Linux updates does not refresh APT or install anything. It reads the
installed packages and a no-change preview.

1. **Check for updates** runs `apt-get update` as a background job.
2. Select the exact packages you want. Search works, for example
   `plexmediaserver`.
3. Review the dialog. It shows versions, marks packages that often need a host
   reboot (kernel, libc, systemd and similar), and asks you to acknowledge the
   disruption and type `APPLY N UPDATES` (or `APPLY 1 UPDATE`).
4. Right before applying, the broker checks again: installed and candidate
   versions, holds, download space with 512 MB of headroom, and a preview that
   must add and remove nothing. It keeps existing configuration files, runs one
   package job at a time, and verifies every final version.

Helix never reboots after package work. Broad or unattended upgrades are not
available.

APT is not transactional. Helix cannot roll back a failed maintainer script or
an update cut off by power loss. Read the job log and use normal `dpkg` and APT
recovery (`sudo dpkg --configure -a`, `sudo apt-get -f install`) if the system
reports a partial configuration.

The broker runs APT as root without switching to the `_apt` user, because the
unit's `NoNewPrivileges` setting blocks that switch.

### Plex updates

Plex installed as `plexmediaserver` from its official APT repository appears in
the list when an update is offered. Updating it may restart Plex and interrupt
streams; your library and settings stay. If Plex came from a standalone `.deb`
without its repository, follow
[Plex's repository instructions](https://support.plex.tv/articles/235974187-enable-repository-updating-for-supported-linux-server-distributions/)
first. The list covers APT packages only, not Docker images, Flatpaks, Snaps, or
hand-installed programs.

## Reboots

If Linux writes `/var/run/reboot-required`, Host says a reboot is needed and
names the packages when the OS lists them. **Reboot host** is beside that
notice, in the Host updates toolbar, and in Settings.

**Reboot now**:

- needs the power permission;
- checks for known active players and running jobs, which block the reboot;
- shows a warning instead of blocking when a server cannot report its player
  count; and
- asks for one confirmation, then starts Linux's normal shutdown at once. It
  cannot be cancelled, and there is no countdown.

**Recurring reboot** runs daily or on chosen weekdays at one host-local time.
The dialog shows the Linux timezone and the next run. A scheduled reboot skips
when players are active, jobs are running, or a player count cannot be checked.
Package updates never trigger a reboot.

### Start after boot

The start-after-boot setting (in Settings and the Security page) changes only the restart
policy of the exact dashboard and gateway containers. It shows a busy state until
the broker verifies both. Recreating the containers later (for example by
re-running the installer) applies the policy in `compose.yaml` again, which is
`unless-stopped`. Each game server has its own **Start on boot** setting.

## Docker cleanup

**Settings → Docker cleanup** shows the space Docker reports for images,
containers, volumes, and build cache, plus Docker's real data folder, the
filesystem that holds it, and its free space. There is no drive picker: Docker
decides that location in its own configuration.

**Clean Docker now** runs in the background and survives a page refresh. It
removes only:

- build cache older than the retention period;
- dangling (untagged) images older than that period; and
- unused networks older than that period.

It keeps every container, running or stopped, every volume, every named image,
and anything Docker considers in use. It therefore frees less than Docker's
"reclaimable" figure. Retention defaults to 7 days and can be set from 1 day to
1 year.

**Schedule** runs the same cleanup on chosen weekdays at one host-local time.
The dialog shows the host timezone and next run. Missed runs do not catch up
after boot. Helix checks its saved unit files before changing or running the
schedule, records the exact last result, and will not start while a host reboot
is pending.

## Updating Helix

### In the dashboard

Helix checks GitHub for a newer release when you open Linux updates, and caches
the answer for 30 minutes. **Check GitHub** checks again now. Only published
`vMAJOR.MINOR.PATCH` releases count; drafts and prereleases are ignored.

**Update Helix** asks you to acknowledge that the dashboard will restart and to
type `UPDATE HELIX`. Then the broker:

1. Confirms the target is still GitHub's latest release and newer than this
   host.
2. Finds the Compose project and its env file from the dashboard container's
   labels. If it cannot, the update stays unavailable with the reason "no
   detected dashboard Compose project".
3. Checks that `/var/lib/helix/updates` has at least 1 GiB free.
4. Downloads `SHA256SUMS` and `helix-source-<version>.tar.gz` over HTTPS from
   GitHub, and checks the archive against the listed SHA-256 (and against
   GitHub's own asset digest when GitHub provides one).
5. Rejects archives with unexpected paths, then unpacks.
6. Records a rollback point: tags the current dashboard and gateway images as
   `helix-rollback`, copies the current `helix-privd` and `helix-terminald`,
   and takes a dashboard state snapshot in
   `/srv/helix/dashboard/backups/helix-update-<time>.db` when it can.
7. Builds the new dashboard, gateway, broker, and terminal images.
8. Starts `helix-finalize-update.service` and restarts.

The finalizer installs the new binaries, restarts `helix-privd` and any active
terminal units, recreates the dashboard and gateway, checks that every other
container on the host is unchanged, and runs `helixctl ready`. If any step
fails, it puts back the previous binaries and images and records the result in
`/var/lib/helix/updates/last-result.json`.

The browser reloads when the new dashboard answers. Game containers, AMP, and
Plex keep running. Helix does not reboot the host.

Follow an update from the server:

```bash
sudo journalctl -u helix-finalize-update -f
```

The updater trusts GitHub HTTPS and the published SHA-256. Releases are not
signed with an independent key. `git pull` is not an update method.

### Manually

Re-run the one-line installer. It downloads the latest release, checks its
checksum, rebuilds, and restarts Helix, keeping your settings and data:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash
```

Pass `--version vX.Y.Z` for a specific release. See
[Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration#check-repair-and-upgrade).

### Rolling back

- **A failed in-app update rolls itself back** to the previous images and
  binaries. Check `last-result.json` and the finalizer log above.
- **After a successful update**, there is no rollback button. The dashboard
  refuses a state database with a newer schema than it understands, so
  installing an older release with `--version` works only when the newer
  release did not change the schema. If it did, you also need the state snapshot
  the updater took (`helix-update-<time>.db`), and that restore is manual and
  untested. See
  [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery#restoring-helixs-own-state).
- The installer keeps the broker binary it replaced as
  `/usr/local/libexec/helix-privd.previous`.

Back up `/srv/helix` and `/etc/helix` off the machine before large upgrades.
