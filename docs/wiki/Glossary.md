# Glossary

Short definitions of the words Helix uses, in alphabetical order. Each entry
links to the page with the details.

**AMP** — A separate game-server manager by CubeCoders. Helix can show and
control AMP instances through AMP's own loopback API, but AMP keeps owning
them. See Imported server below.
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace)

**Analysis root** — A folder listed in `analysis_roots` in the broker config
that the space analyzer may read. Usually `/`. Read-only.
[Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files)

**Backup trash** — Where a server's deleted backups go, so you can undo. It
empties itself after 7 days by default; each server can choose 0 (never) to
365 days.
[Backups](https://github.com/Riqqqque/Helix/wiki/Backups)

**Broker** — `helix-privd`, the root service that does everything needing
root: Docker, files, firewall, packages, reboots, updates. It only accepts a
fixed list of typed requests.
[How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works)

**Capability** — A named permission such as `games.manage` or
`terminal.open`. The server checks it on every request. The owner holds all
of them.
[Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture#capabilities)

**CSRF proof** — A second secret sent with the session cookie on every
protected request, in the `X-Helix-CSRF` header. It stops other sites from
acting with your cookie.
[Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery)

**Deploy** — One job that applies a server's staged changes: optional backup,
stop, install, start, wait until ready, and put the old files back if it fails.
[Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes)

**Gateway** — The nginx container in front of `helixd`. It accepts only the
configured host name, browser origin, and client networks.
[Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture)

**Genome** — A planned portable export format for Helix settings and data.
Design only; nothing in Helix reads or writes it yet.
[How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works#what-is-still-design-only)

**helix-terminald** — The optional service behind the Terminal page. It runs
as one normal Linux user and gives each connection a login shell.
[Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal)

**helixctl** — The local command-line tool for setup tokens, health checks,
state snapshots, and Strand projects.
[Development and Testing](https://github.com/Riqqqque/Helix/wiki/Development-and-Testing)

**helixd** — The dashboard server. It serves the web app and API, checks
sessions and capabilities, and runs unprivileged in a read-only container.
[Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture)

**Home** — Your own dashboard page made of widgets you place on a grid. You
can keep up to eight Homes.
[Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home#home)

**Hook** — A card for a service already on your host, such as Plex, Jellyfin,
Tailscale, Docker, or AMP. It shows state and offers exact controls; the
service keeps its own data.
[Hooks and Terminal](https://github.com/Riqqqque/Helix/wiki/Hooks-and-Terminal)

**Imported server** — A game server another
manager owns, such as an AMP instance, that Helix shows and partly controls.
Helix does not rewrite its files. Copying one into Helix makes a new native
server.
[Server Migration](https://github.com/Riqqqque/Helix/wiki/Server-Migration)

**Job** — Work that outlives one request, such as an install, backup,
update, or deploy. It has a stage and percent, survives a page reload, and its
record survives a broker restart.
[How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works#jobs)

**Keep rules** — A server's backup limits: keep the newest N backups (up to
50) and anything newer than D days (up to 365). Older copies move to backup
trash.
[Backups](https://github.com/Riqqqque/Helix/wiki/Backups)

**Live backup** — A backup of a running Minecraft server. Helix pauses
autosave, flushes the world, archives it, and turns autosave back on, so
players stay online.
[Backups](https://github.com/Riqqqque/Helix/wiki/Backups)

**Managed root** — A folder listed in `managed_roots` in the broker config.
Helix only creates, edits, uploads, renames, or trashes files inside these.
[Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files)

**Native server** — A game server Helix created and owns. It runs in its own
Docker container, managed through the broker.
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace)

**Operation lock** — The per-server lock that keeps actions from colliding.
Reads need none, file writes share it, and lifecycle actions, backups,
restores, updates, transfers, and deploys hold it alone.
[How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works#operation-locks)

**Owner** — The one account created during setup. It holds every capability.
[Getting Started](https://github.com/Riqqqque/Helix/wiki/Getting-Started)

**Ready marker** — A `.helix-ready` file that a non-Minecraft game runtime
writes in the server's data folder once the dedicated server is installed and
running. Minecraft readiness comes from its status ping instead.
[Servers and Marketplace](https://github.com/Riqqqque/Helix/wiki/Servers-and-Marketplace)

**Recoverable trash** — Two things share this name. In the file manager it is
the `.helix-trash` folder at the top of a managed root. In **Settings → Helix
data** it is the list of removed native servers you can restore or delete
forever. Deleted backups use the separate backup trash.
[Storage and Files](https://github.com/Riqqqque/Helix/wiki/Storage-and-Files),
[Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home#settings)

**run_uid** — The Linux user ID each native server runs as, picked between
20000 and 59999. Its container and files use that ID, keeping servers apart.
[Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture)

**Sequence** — A planned declarative package format for installing and
running games. Design only; there is no parser or engine.
[How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works#what-is-still-design-only)

**Server token** — A server API token: a secret for scripts and tools that
grants chosen permissions on chosen servers, never the host itself.
[Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)

**Session** — Your signed-in browser. It ends after 30 minutes idle or 8 hours
total unless **Stay signed in** is on.
[How Helix Works](https://github.com/Riqqqque/Helix/wiki/How-Helix-Works#sessions)

**Staged change** — A file change made while a server runs. It waits in a
broker-owned store outside the server folder until you deploy it.
[Deploying File Changes](https://github.com/Riqqqque/Helix/wiki/Deploying-File-Changes)

**Start after the host boots** — A per-server switch that brings a native
server back after Linux or Docker restarts. On by default for new servers.
[Dashboard and Home](https://github.com/Riqqqque/Helix/wiki/Dashboard-and-Home#settings)

**State database** — `helix-state.db`, the SQLite file that holds the owner,
sessions, preferences, Home layouts, tokens, machines, and Strands.
[Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture#data-stores)

**State root** — The broker's folder of records for native servers
(`native.state_root`), holding manifests, `jobs.json`, staged changes, console
history, and server trash.
[Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture#broker-roots)

**Strand** — An extension you install from a `.strand.zip`. It adds a page or
Home widget that runs in a sandboxed frame and can only use the host calls it
declares.
[Building Strands](https://github.com/Riqqqque/Helix/wiki/Building-Strands)

**Token vault** — An encrypted copy of each server token, kept so the owner
can view it again. Its key lives in `secrets/token-vault.key`, apart from the
state database.
[Server API Tokens](https://github.com/Riqqqque/Helix/wiki/Server-API-Tokens)
