# Troubleshooting and FAQ

Practical fixes for the problems people actually hit, using the messages Helix
and its installer print. Start with the health check; it finds most install
problems in one go:

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash -s -- --check
```

`--check` changes nothing and downloads no release. The installer copy in
`/opt/helix/releases/<version>/scripts/install.sh` works too.

## Where the logs are

| What | Command |
| --- | --- |
| Host broker (servers, files, host actions) | `sudo journalctl -u helix-privd -n 200 --no-pager` |
| In-app Helix update | `sudo journalctl -u helix-finalize-update -n 200 --no-pager` and `sudo cat /var/lib/helix/updates/last-result.json` |
| Dashboard (`helixd`) | `sudo docker logs --tail 200 server-dashboard` |
| Gateway (nginx) | `sudo docker logs --tail 200 server-dashboard-gateway` |
| A game server | The server's **Console** tab, or `sudo docker logs --tail 200 helix-game-<id>` |
| Browser terminal | `sudo journalctl -u 'helix-terminald@*' -n 100 --no-pager` |
| Docker itself | `sudo journalctl -u docker -n 200 --no-pager` |

Follow a log live by adding `-f`. The server ID is in the server page's URL.

## Installer failures

| Message | What to do |
| --- | --- |
| `run this as root` | Run it with `sudo`, as in the one-line command |
| `no terminal for the question …; re-run with --yes` | The installer was run without a terminal (cloud-init, CI). Add `--yes` and the options you want, such as `--lan-ip`, `--cidr`, `--install-docker` |
| `Helix needs systemd as the init system` | Helix needs systemd as PID 1. It will not install inside most containers or on WSL without systemd |
| `Helix supports 64-bit x86 and ARM; this is …` | 32-bit systems are not supported |
| `the '…' command is missing` or `curl is missing` | Install that package with your package manager and re-run |
| `install Docker Engine with the Compose plugin …` | Answer yes to the Docker question, pass `--install-docker`, or install Docker yourself |
| `Docker installed, but 'docker compose' is still unavailable` | Install the Docker Compose v2 plugin (`docker-compose-plugin`) |
| `could not read the latest release from GitHub` | The server could not reach the GitHub API, or hit its rate limit. Check DNS and outbound HTTPS, or pass `--version vX.Y.Z` |
| `the downloaded archive does not match the published SHA-256 checksum` | The download was damaged or altered. Re-run. If it keeps failing, do not work around it; open an issue |
| `this server has no private network address` | The server has only a public address. Join it to Tailscale or a VPN, then pass `--lan-ip` with that private address |
| `… is not a private address` / `… is not an address of this server` | Pass an address that is both private and assigned to this machine (`ip -4 addr`) |
| `the saved address … is no longer on this server; re-run with --reconfigure` | The server's address changed, usually through DHCP. Give it a fixed address or DHCP reservation, then re-run with `--reconfigure` |
| `port … is already in use on this server; choose another with --port` | Another program uses 3100. Pick a free port with `--port` |
| `… is too broad; use your LAN subnet` | `--cidr` must be `/8` or narrower |
| `storage folder … does not exist` / `… cannot be a Helix storage folder` | `--storage` folders must exist and cannot be system folders such as `/` or `/etc` |
| `the terminal cannot run as root`, `user … does not exist`, `… must not be in the helix-broker group` | `--terminal-user` must be an existing ordinary login that is not in `helix-broker` |
| `the host broker did not start; see the log above` | Read the broker log lines printed above the error. See [Broker will not start](#broker-will-not-start) |
| `could not create the owner setup token` | The token command failed; its output is printed above. Check free disk space and the folder `/srv/helix/dashboard/data` |
| `the dashboard did not answer at … ; run '… --check'` | Run `--check` and read which piece is unhealthy |
| A build step fails | Usually out of disk or memory. The first build needs about 10 GB free and 4 GB RAM. Check `df -h` and `free -h`, then re-run |

Re-running the installer is safe. It keeps your settings, data, and servers.

## Cannot reach the dashboard

Use the exact address the installer printed, such as `http://192.0.2.10:3100`.

| Symptom | Cause | Fix |
| --- | --- | --- |
| `421 Misdirected Request` | You used a hostname, `localhost`, or another address. The gateway accepts only the exact configured Host | Use the IP address from `HELIX_GATEWAY_HOST` in `/opt/helix/helix.env` |
| `403 Forbidden` on every page | Your device is outside the allowed client network | Check your device's address against `HELIX_ALLOWED_CLIENT_CIDR`. Re-run with `--reconfigure --cidr …` to change it |
| `403` only when saving | The browser's Origin does not match `HELIX_BROWSER_ORIGIN`, for example through a proxy or a different address form | Open Helix at exactly `http://<address>:<port>` |
| Connection times out | UFW, another firewall, or a different network | Allow the port: `sudo ufw allow from 192.0.2.0/24 to any port 3100 proto tcp`, or re-run with `--open-firewall`. Check you are on the same LAN or VPN |
| Connection refused | The containers are down or the address moved | Run `--check`. If it says the address is no longer on this server, re-run with `--reconfigure` |
| Works from the laptop, not from the server itself | `127.0.0.1:3100` is not published | Use the LAN address on the server too |
| "Host management is temporarily unavailable." | The dashboard cannot reach the broker | `sudo systemctl status helix-privd`, then see the next section |

## Broker will not start

The dashboard still loads, but servers, files, and host pages say host
management is unavailable. Read `sudo journalctl -u helix-privd -n 50`.

| Log message | Fix |
| --- | --- |
| `the broker config must be root-owned and inaccessible to other users` | `sudo chown root:root /etc/helix/privd.json && sudo chmod 600 /etc/helix/privd.json` |
| `broker paths must be absolute and at least one managed root is required` | Fix the paths in `privd.json`, and keep at least one `managed_roots` entry |
| A JSON parse error, or `unknown field` | A typo or an unsupported key in `privd.json`. Every key is checked; see [the broker reference](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration#broker-privdjson) |
| `managed root … no longer exists; skipping it` | A warning only. Remove the folder from `managed_roots` to silence it |
| `none of the configured managed roots exist` | At least one `managed_roots` folder must exist |
| `another helix-privd process is already listening` | A second broker is running. `sudo systemctl restart helix-privd` and check for stray processes |
| `console history retention must use 16 MiB to 32 GiB across 2 to 256 files` | Fix `console_history_max_bytes` or `console_history_files` |
| `backup trash retention must be at most 365 days, or 0 to keep deleted backups` | Fix `backup_trash_retention_days` |
| `the Docker executable is unavailable` | Set `native.docker_binary` to the output of `command -v docker` |

After fixing, run `sudo systemctl restart helix-privd`. Game servers keep running
while the broker restarts.

## Lost owner access

There is no password reset. Every account change needs the current password, and
once an owner exists, `setup-token` refuses with
`an owner account already exists; setup tokens are permanently disabled`.

If you are still signed in somewhere, change the password in **Settings** now.
Otherwise you have two ways back:

1. **Restore a state snapshot** taken before you lost the password. See
   [Restoring Helix's own state](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery#restoring-helixs-own-state).
   The in-app updater leaves snapshots in `/srv/helix/dashboard/backups`.
2. **Start the dashboard state over.** Stop the dashboard, move
   `/srv/helix/dashboard/data` aside, create an empty replacement owned by
   `10001:10001` with mode `0700`, and re-run the installer with `--new-token`:

   ```bash
   sudo docker stop server-dashboard-gateway server-dashboard
   sudo mv /srv/helix/dashboard/data /srv/helix/dashboard/data.old
   sudo install -d -m 0700 -o 10001 -g 10001 /srv/helix/dashboard/data
   curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash -s -- --new-token
   ```

   You lose the dashboard's own data: accounts, sessions, Home layouts and
   preferences, server API tokens, audit history, and metrics. Game servers,
   worlds, backups, and firewall records live under `/srv/helix/instances`,
   `/srv/helix/backups`, and `/srv/helix/state`, so they stay. Create new API
   tokens afterwards.

## A server will not start

Open the server's **Console** tab first; the last lines usually say why.

| Message | Cause and fix |
| --- | --- |
| `The N MiB memory allocation was exhausted. Increase the server memory and try again` | The game ran out of its memory limit (Java heap or the container limit). Raise **Memory** in the server's settings |
| `port N is already in use on this host by a program Helix does not manage, such as an AMP server or another game panel` | Stop that program or choose another port. Find it with `sudo ss -lntup 'sport = :N'` |
| `another Helix server already uses port N` | Pick a different port, or stop and change the other server |
| `port N is in use by another program on this host` | Same as above, for the main game port |
| An AMP instance is named as the port owner | Change the port in AMP (stop the instance, Configuration, Server Settings or Portals, Apply). Helix will not edit AMP |
| `Minecraft kept running but did not open port N within S seconds` | The server is slow to start or stuck. Read the startup output included in the message. Large modpacks need more memory and time |
| `Execution container is missing` | The server's Docker container was removed outside Helix |
| `… dedicated server files are not installed yet` | The first download did not finish. Check disk space and the job log, then try again |

If the machine itself runs low on memory, Linux may stop a game to free memory.
See [Memory overcommit warning](#memory-overcommit-warning).

## "Busy: …" and jobs waiting in line

Only one exclusive operation (backup, restore, update, install, copy, start,
stop) runs on a server at a time. File reads never wait, and file uploads can
run side by side.

- The server page shows `Busy: <operation>. New actions wait in line until it
  finishes.`
- Background jobs wait their turn for up to 30 minutes. Their stage says
  `Waiting for <operation> (started N seconds ago)`.
- A direct call that cannot wait gives up after 8 seconds with
  `<operation> (started …) is still running on this server, so <action> was not
  started. Try again when it finishes.`

Let the first job finish. Large backups and first-time Steam downloads can take
many minutes.

If the broker restarted during a job, that job is marked failed with
`Helix restarted while this job was at "<stage>". Check the server before trying
again`. Look at the server, then run the action again. The message
`The host broker is busy.` means too many requests at once; wait a moment.

## Staged changes were not applied

Files uploaded through the server API while a server runs are staged and
installed on the next start, restart, or **Deploy**. See
[Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes).

| Message | What to do |
| --- | --- |
| `… is ready, but its staged file changes were not installed: …` | The server started anyway. Read the reason, fix it, then restart or **Deploy** again |
| `staged changes could not be installed (…). Nothing was changed; the changes are still staged` | Fix the named problem (often a missing folder or a full disk) and try again |
| `… putting back the others also failed (…). Check the Files trash` | Some files moved. Check the server's Files trash (`.helix-trash`) for the originals |
| `file revision conflict; the staged change was replaced or installed` | Someone changed or installed that file meanwhile. List pending changes or reread the file |
| `the list of staged changes is damaged; discard pending changes and stage them again` | Use **Discard** on the server page, then upload again |
| `the pending store is not private to Helix; staged changes were not used` | The folders under `/srv/helix/state/instances/pending/` must be owned by root with no group or other permissions. Something changed them; restore root ownership and mode `0700` |

## Backups are slow or large

- **Install pigz.** With `/usr/bin/pigz` present, Helix compresses on half the
  CPU cores (at most 8). `sudo apt-get install pigz`. Archives stay normal
  `.tar.gz` files.
- **Running Minecraft servers back up live.** Helix pauses autosave, flushes
  the world, archives, and turns autosave back on. If the console does not
  answer, it stops, backs up, and starts instead. The backup result's `mode` is
  `live`, `stopped`, or `offline`.
- **Minecraft backups skip** `logs/`, `crash-reports/`, `cache/`,
  `plugins/.paper-remapped/`, and the Files trash.
- **Set keep rules** on the server's Backups tab so old backups are trashed
  automatically.
- Backups live in `/srv/helix/backups`, on the same disk as the servers. Copy
  important ones elsewhere.

See [Backups](https://github.com/Riqqqque/Helix/wiki/Backups).

## The disk is filling up

- **Backup trash** empties itself: deleted backups are removed 7 days after
  deletion by default. Each server can change that (`0` = never). Helix checks
  every hour and after each backup. To empty it now, use the purge action on the
  Backups tab, or the API's `purge_backup_trash_older_than` with `0` days.
- **Files trash** (`.helix-trash`) and **Removed and hidden** servers do not
  expire. Empty them yourself. Permanently deleting a removed server needs its
  exact name and deletes its world files, backups, and console history.
- **Docker** build cache and old images: **Settings → Docker cleanup**.
- **Console history** is capped per server (512 MiB by default).
- Find what is big with **Storage** analysis.

## Memory overcommit warning

The Servers page and the memory setting warn when the memory limits of all your
servers add up to more than the machine has: "Servers may use … of memory
together, but this machine has …". That is allowed, and works
while the servers are not all busy at once. If they are, Linux stops one of
them to free memory. Lower the largest limits, or keep some servers stopped.
See [Game Hosting and Capacity](https://github.com/Riqqqque/Helix/wiki/Game-Hosting-and-Capacity).

## AMP and imported servers

AMP stays in charge of its own servers. Through AMP's local API, Helix shows
their status, players, ports, and console, and offers the lifecycle actions it
can verify. It does not:

- force-kill AMP instances (stop them in Helix or kill them in AMP);
- change AMP settings or files (AMP-only settings stay in AMP);
- treat AMP Idle (sleep) as online; Helix offers **Start** to wake it; or
- turn an imported AMP or Pterodactyl server into a Helix server. Use **Copy
  into Helix** to make a new Helix server from it.

**Hide** and **Forget here** only affect this browser's list. Only Minecraft AMP
instances appear in the AMP inventory; for other AMP games, copy from the
instance folder. See
[Server Migration](https://github.com/Riqqqque/Helix/wiki/Server-Migration).

AMP connection errors and their fixes:

| Message | Fix |
| --- | --- |
| `AMP credentials must be root-owned and inaccessible to other users` | `sudo chown root:root /etc/helix/amp.json && sudo chmod 600 /etc/helix/amp.json` |
| `AMP endpoint must remain on loopback` | `endpoint` must be a `127.0.0.1` address |
| `AMP credentials are unavailable` / `AMP credential file is invalid` | Check the file exists and is valid JSON with `username` and `password` |

## Updater problems

| Message | What to do |
| --- | --- |
| `GitHub could not be checked yet: …` | The server cannot reach `api.github.com`. Check DNS and outbound HTTPS, then **Check GitHub** |
| `… no detected dashboard Compose project …` | The dashboard was not started by Compose, or was recreated by hand. Re-run the installer, which starts it with Compose |
| `This host is already on Helix …` | Nothing to do |
| `GitHub latest is …, not …; check again before applying` | A newer release appeared. Choose **Check GitHub**, then apply again |
| `another Helix update is already running` | Wait for it. Follow `journalctl -u helix-finalize-update -f` |
| `the Helix update staging filesystem does not have a full gigabyte free` | Free space on the filesystem holding `/var/lib/helix` |
| `the … tool is unavailable` | Install the named tool (`curl`, `tar`, `timeout`, `docker`, `systemctl`) |
| `the Helix source archive failed its SHA-256 digest` | The download did not match. Try again later. Do not bypass it |
| `…; Helix restored the previous dashboard, gateway, and broker` | The update failed and rolled back. The reason is first in the message and in `last-result.json` |

If the dashboard does not come back after an update, run `--check`, read the
finalizer log, and re-run the one-line installer to rebuild the latest release.

## Collecting information for a bug report

Open an issue at
[github.com/Riqqqque/Helix/issues](https://github.com/Riqqqque/Helix/issues).
For a security problem, use
[private vulnerability reporting](https://github.com/Riqqqque/Helix/security/advisories/new)
instead.

Include:

- the Helix version (the Helix update card under **Host → Linux updates** shows
  it) and how you installed it;
- the OS and version (`cat /etc/os-release`), CPU architecture (`uname -m`), and
  Docker version (`docker --version; docker compose version`);
- the output of `--check`;
- the exact error message and what you clicked or called;
- the relevant log lines from the table at the top, trimmed to the failure.

Before posting, remove passwords, setup tokens, API tokens, cookies, private and
public IP addresses, hostnames, storage paths, player names, and world data.
Replace addresses with placeholders such as `192.0.2.10`.

## FAQ

### Is it safe to put Helix on the internet?

No. Helix is a private-LAN release, serves plain HTTP, has no multi-factor
sign-in, and has not had a public-network security review. Keep the dashboard on
your LAN and use Tailscale or a VPN from elsewhere. Forwarding a **game** port
for players is fine; forwarding the dashboard port is not.

### Does it replace AMP or Pterodactyl?

It can, for the games it supports, but it does not take over their servers. You
can see AMP's Minecraft servers in Helix and keep running them in AMP, or copy a
server from AMP or Pterodactyl into a new Helix server. The original stays
untouched.

### Which games does it run?

Minecraft (Paper, Purpur, Folia, Leaves, Pufferfish, Fabric, Quilt, Forge,
NeoForge, Vanilla, Pumpkin, or your own JAR), Hytale, Valheim, V Rising,
Terraria and tModLoader, Palworld, Satisfactory, Project Zomboid, 7 Days to Die,
Rust, Sons of the Forest, Factorio, Don't Starve Together, and Vintage Story.
See [Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace).

### Where is my data stored?

| What | Where |
| --- | --- |
| Game server folders and worlds | `/srv/helix/instances/` |
| Game server backups | `/srv/helix/backups/` |
| Server definitions, console history, staged changes | `/srv/helix/state/instances/` |
| Dashboard accounts, preferences, tokens | `/srv/helix/dashboard/data/` |
| Settings | `/opt/helix/helix.env`, `/etc/helix/privd.json` |

The full layout is in
[Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration#install-layout).

### Does Helix send data anywhere?

There is no account and no telemetry. Helix contacts the services you use:
GitHub for update checks, game and runtime downloads, Modrinth, CurseForge when
you saved an API key, weather for your city, and AMP on loopback.

### Do game servers stop when I close the browser or update Helix?

No. They run in their own containers. Closing the browser, restarting the
dashboard or broker, and in-app updates leave them running. A host reboot stops
them; each server's **Start on boot** setting decides whether it comes back.

### How do I uninstall?

```bash
curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash -s -- --uninstall
```

This removes Helix's services, binaries, and dashboard containers and keeps game
servers, worlds, backups, and settings. Add `--purge` to delete those too; it
asks you to type `DELETE`. See
[Installation and Configuration](https://github.com/Riqqqque/Helix/wiki/Installation-and-Configuration#uninstall).
