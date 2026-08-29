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
country centroids. Player pins are sockets whose local port is a published game
port. Outbound pins are the rest. Helix does not geolocate private, loopback,
link-local, multicast, or CGNAT (`100.64/10`) addresses, and it does not send
those remote IPs to the browser. If the WAN address is missing or not globally
routable, destination countries still plot without a host pin.

## Host services and processes

Host displays bounded service and process tables with pagination rather than a
single page-height list. Hover/focus the information icons for definitions; the
tooltip is rendered above card clipping and stays inside the viewport.

**Helix footprint** counts the dashboard, gateway, and broker only. Game servers
are shown separately so their memory is not misrepresented as dashboard cost.

Host also lists every Docker container on the machine, with CPU and memory when
Docker reports them. Start, stop, and restart require the exact container name.
Helix dashboard and gateway containers stay protected.

## System packages

Opening System updates does not refresh APT or install anything. **Check for
updates** starts a separate `apt-get update` job. Select exact candidates before
Apply; the confirmation dialog shows package versions and requires disruption
acknowledgement plus an exact phrase.

Immediately before Apply, the broker rechecks installed/candidate versions,
holds, download space with headroom, and an APT simulation. It rejects any
removal or new package, preserves current conffiles, serializes package work,
verifies the final versions, and never reboots Linux automatically.

APT is not transactional. Helix does not claim it can roll back a failed package
maintainer script or power loss. Read the job log and use normal dpkg/APT recovery
when the operating system reports a partial configuration.

Helix self-update remains unavailable. A future updater must use signed,
digest-pinned artifacts, pre-migration data/config backup, staged health checks,
and automatic rollback; `git pull` is not an updater.

## Start after boot and reboot

The Settings toggle changes only the exact dashboard and gateway container
restart policies and shows a busy state until the broker verifies both.

Reboot now is actually a cancellable 10–300 second schedule, giving the operator
time to cancel. It requires the exact hostname and disruption acknowledgement
and checks active players and jobs. Recurring reboot supports daily or selected
weekdays at one host-local time; the UI shows the Linux timezone and next run.
Helix never couples package updates to an automatic reboot.
