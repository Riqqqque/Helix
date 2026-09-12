# Network, Host, and Updates

## Network evidence

The Network page keeps local listeners, Docker publications, game ports, UFW
state, and outside reachability separate. A process listening locally does not
prove Docker published it; a Docker publication does not prove UFW or the router
permits it; an allow rule does not prove a remote player can connect.

Named TCP/UDP rules can cover one port or a bounded range. Helix owns only rules
with its exact opaque marker and durable record, verifies every change, and can
Undo a recent deletion. It does not delete unknown rules or change UFW defaults.

If UFW is installed but inactive, **Enable UFW safely** asks for the current SSH
TCP port and the literal confirmation `ENABLE UFW`. Helix first proves that port
is listening, stages its exact SSH safety rule, enables UFW, and verifies both.
Failure triggers an attempt to restore the prior inactive state. Use a
disposable host for first validation; this remains a host-wide firewall change.

Helix cannot open a router, bypass CGNAT, or prove internet reachability.

## Globe

The Globe page (and matching Home widget) maps established public TCP sockets to
country centroids. Game-port pins are sockets whose local port is a published
game port, including pings and join attempts, not only logged-in players.
Outbound pins are the rest. Helix does not geolocate private, loopback,
link-local, multicast, or CGNAT (`100.64/10`) addresses, and it does not send
those remote IPs to the browser. If the WAN address is missing or not globally
routable, destination countries still plot without a host pin.

An open public game port is found by internet scanners even if you never shared
the address. That is not a Helix leak of the IP. Turn off Helix public access
for LAN or Tailscale only, and for Minecraft turn on **Whitelist** so unapproved
accounts cannot play.

The Home Globe widget fills its card (the map covers the tile; poles or the date
line may be cropped). The Globe page still shows the full 2:1 world.

## Host services and processes

Linux updates are at the top of Host, above services and processes. Host still
displays bounded service and process tables with pagination rather than a
single page-height list. Hover/focus the information icons for definitions; the
tooltip is rendered above card clipping and stays inside the viewport.

**Helix footprint** counts the dashboard, gateway, and broker only. Game servers
are shown separately so their memory is not misrepresented as dashboard cost.

Host also lists every Docker container on the machine, with CPU and memory when
Docker reports them. Empty published ports are normal. Start, stop, and restart
require the exact container name. Helix dashboard, gateway, and native game
containers stay protected. Use Servers to restart a Helix game.

## System packages

Linux updates sit at the top of Host. Opening that page does not refresh APT or
install anything. **Check for updates** talks to the signed package mirrors.
Select exact candidates before Apply. The confirmation dialog shows versions,
says when a package often needs a host reboot, and requires disruption
acknowledgement plus an exact phrase.

Immediately before Apply, the broker rechecks installed/candidate versions,
holds, download space with headroom, and a no-add/no-remove preview. It rejects
any removal or new package, preserves current config files, serializes package
work, verifies the final versions, and never reboots Linux.

If Linux later writes `/var/run/reboot-required`, Host says a reboot is needed
and names the packages when the OS listed them. Use **Reboot host** beside the
notice, in the Host updates toolbar, or in Settings. Confirm **Reboot now** in
the dialog; there is no hostname to type or countdown. Helix does not reboot as
a side effect of applying packages.

APT is not transactional. Helix does not claim it can roll back a failed package
maintainer script or power loss. Read the job log and use normal dpkg/APT recovery
when the operating system reports a partial configuration.

The host broker runs APT without dropping to the `_apt` user because systemd
`NoNewPrivileges` blocks that seteuid. The broker is already root inside its
unit sandbox.

Helix self-update checks GitHub for a newer `vMAJOR.MINOR.PATCH` release when
you open Linux updates, and **Check GitHub** forces a fresh look. **Update
Helix** downloads the SHA-256-pinned source archive, rebuilds only Helix
dashboard/gateway images, and replaces helix-privd and helix-terminald. It
health-checks and restores those on failure. The browser reloads when the new
dashboard answers. `git pull` is not an updater. Game containers, AMP, and Plex
stay running. Independently signed Helix keys remain a public-internet gate.

## Start after boot and reboot

The Settings toggle changes only the exact dashboard and gateway container
restart policies and shows a busy state until the broker verifies both.

Reboot now checks active players and jobs, then asks for one confirmation.
Confirming starts Linux's normal shutdown immediately; it cannot be cancelled.
Save your work first. Recurring reboot supports daily or selected
weekdays at one host-local time; the UI shows the Linux timezone and next run.
Helix never couples package updates to an automatic reboot.

If a server cannot report its player count, a manual reboot shows a warning
instead of requiring you to repair that integration first. Confirming still
disconnects any players. Known active players and running jobs block reboot;
recurring schedules also skip when player counts cannot be verified.

## Plex updates

Plex installed as `plexmediaserver` appears in the Linux package list when its
official APT repository offers an update. Choose **Check for updates**, search
for `plexmediaserver`, select it, then review and apply. Updating Plex may restart
its service and interrupt streams; it does not replace your library or settings.
Helix rechecks the exact version before installing and verifies it afterward.

If Plex was installed from a standalone DEB without its repository, follow
[Plex's repository instructions](https://support.plex.tv/articles/235974187-enable-repository-updating-for-supported-linux-server-distributions/)
first. This list covers installed APT packages, not Docker images, Flatpaks,
Snaps, or manually downloaded programs. Container updates belong to their
container manager. Plex beta releases may differ from the public repository.
