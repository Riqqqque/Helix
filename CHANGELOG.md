# Changelog

This file records user-visible and operator-visible changes. Numbered GitHub
releases pin a source archive with SHA-256 checksums. That is a private-LAN
release, not a public-internet support promise.

## Unreleased

### Added

- Settings → Catalogs stores an owner CurseForge API key on the host so marketplace
  search and “Start with a modpack” can use `api.curseforge.com`. Helix never
  ships a CurseForge secret and never shows the saved key again.
- Removed native servers can be permanently deleted from **Removed and hidden**
  or Settings → Helix data after typing the exact name. That wipe includes world
  files, Helix backups, and console history. Hidden AMP connections can be
  forgotten in this browser from the same list without touching AMP.
- Native create can cap container CPU as well as RAM. Overview can change either
  later. `0` means no extra CPU cap.
- V Rising can list on the in-game server browser (EOS and Steam, on by default)
  and can request UDP public Direct Connect the same way Minecraft does for TCP.
- Valheim and Terraria can request public setup at create. Overview can set up
  or remove Helix-owned public access for native servers.
- Creating a Minecraft, V Rising, Valheim, or Terraria server now shows a
  spinner, percent, and elapsed time while the download and first boot run.
  First Steam installs still often take 10–30 minutes; later creates reuse the
  runtime.

### Changed

- Globe and Join copy treat game-port lines as pings and join attempts, and say
  scanners can find an open public port without anyone sharing the IP.

- Host puts Linux updates at the top, with Check for updates as the main action.
  Copy talks about a preview of what would change instead of a "simulation", and
  it says when Linux needs a host reboot. Helix still never reboots for you.
- Overlay windows keep a real inset so titles, copy, forms, and actions no
  longer sit on the border.
- Marketplace search retries a failed catalog call and keeps the last good
  results on screen. CurseForge now uses the official API with the key from
  Settings → Catalogs instead of scraping the public website.
- Copy on join addresses, Terminal, and Home tiles flips to Copied so you can
  tell it landed.
- HTTPS calls Helix makes as itself now send `Helix/1.0.0` in the User-Agent.
- Settings → Helix data is a full-width card with real padding, stacked server
  rows, and plain-language copy for start-after-boot and dismissed notices.
  Create-server and the server page use the same start-after-boot wording.

### Fixed

- Catalogs no longer says a CurseForge CDN block will "clear." That response is
  this host's public IP getting refused, usually a VPS or VPN exit.
- CurseForge console pastes can include hidden unicode, fullwidth `$`, page
  text, or docker `$$` wrapping. Helix pulls out the `$2a$` key and stores that.
- Home Globe fills the widget instead of sitting in a letterboxed 2:1 box. The
  Globe page still shows the whole world.
- Checking for Linux updates no longer dies on APT's HTTPS helper (`seteuid 42`
  / `Method https has died`). The host broker keeps APT as root inside its
  existing sandbox instead of letting APT drop to `_apt`.
- Overview and Host process count is Linux processes (thread groups), not
  kernel threads. `/proc/loadavg` still supplies the thread total, shown as a
  subtitle. On this class of host that is typically a few hundred processes and
  a couple thousand threads.
- Storage use percent accepts findmnt's number or `"12%"` form, and falls back
  to used/size when the percent field is missing, so full-disk warnings still
  fire.
- Server list player totals stay blank when Helix could not verify a player
  query, instead of showing `0`.
- The Docker page will not start, stop, or restart native `helix-game-*`
  containers. Those go through Servers so the stop is 45 seconds and
  health-checked.
- Open in Settings → Helix data now jumps to that server, not just the Servers
  list. Show them again also refreshes the bell menu in the same tab.
- Arranging pages, hiding pages, Home layouts, colors, and the refresh interval
  no longer snap back to factory defaults if you reload before Helix finishes
  saving. This browser keeps the unsaved change and retries.

## 1.0.0 - 2026-08-29

### Added

- Helix 1.0.0 is the first numbered private-LAN release. Security and Overview
  show that version instead of an alpha prerelease identifier.
- Host → System updates checks GitHub for a newer Helix release and can apply it
  from a SHA-256-pinned source archive. Opening that page refreshes the GitHub
  check. After apply, the browser reloads when the new dashboard answers. The job
  rebuilds only the dashboard and gateway, replaces helix-privd and helix-terminald,
  health-checks, and restores those if the new release does not come up. It does
  not use git pull, does not reboot Linux, and does not replace game containers,
  AMP, or Plex.
- GitHub tags `vMAJOR.MINOR.PATCH` publish `helix-source-*.tar.gz` and
  `SHA256SUMS` from the tagged commit.

### Changed

- Product copy calls this a private-LAN release. Public-internet exposure is
  still unsupported. Independent security review, signed-key provenance beyond
  GitHub attestations, and the remaining clean-host matrices stay open.

- One-click Tailscale and Jellyfin installs now show the exact files Helix will
  write, and a failed job names the path, what is wrong with it, and how to
  continue. Try again re-checks the host. Those installers still only accept the
  built-in hook ID and the publisher's signed repository.
- Hooks and Security feel quicker. Those pages no longer wait on a full Host
  status, Network inventory (listeners, UPnP), Docker `stats`, or Minecraft
  port-candidate scan just to fill a few cards. Independent probes run together.
  Hovering the nav starts the page request. Recheck keeps the last cards on
  screen. The Docker hook card lists containers from `docker ps`; open the
  Docker panel when you want live CPU and memory.
- New dashboards poll host metrics every five seconds. A saved one-second
  interval is unchanged. Overview still loads Helix container integration
  when you open Overview or Settings, not on every other page.

### Fixed

- Installing Tailscale or Jellyfin from Hooks no longer dies on the first step
  with "the repository directory is not a root-owned real directory." helix-privd
  uses umask `0007`, so `/run/helix/hook-installs` (and a missing APT keyring
  directory) could be created group-writable and then fail the safety check
  before Helix could chmod it. The installer now creates those directories,
  sets `0700` / `0755`, and only then verifies them. Tmpfiles also declares
  `/run/helix/hook-installs`.

- AMP Idle/sleep is no longer shown as online. Sleeping instances keep their
  memory limit visible, use Start to wake the game, and do not offer Restart as
  the main action. Stopped, starting, failed, and AMP-manager-stopped states
  are labeled as themselves.
- Overview and Host container lists accept Docker rows with empty ports or
  images, skip one bad name instead of hiding every container, and tell helix-privd
  it may write the Docker engine socket.
- First-time V Rising, Valheim, and Terraria creates can build their runtime
  images while helix-privd has a read-only home. Docker CLI/buildx state now
  lives under Helix native state instead of `/root/.docker`.
- Homarr import keeps Homarr's board order and tile width, adds shortcuts onto
  the Home you are already editing, and no longer alphabetizes the catalog.
- Server-list Restart and Open sit in their own actions column instead of
  crowding the TPS value.
- Helix no longer assigns or public-maps ports AMP still has in instance files.
  The error names the AMP instance when Helix can see it and tells you the AMP
  clicks to free the number: stop the instance, Configuration → Server Settings
  / Portals, change the port, Apply. Helix still will not edit AMP files. If
  the AMP instance is already gone and only a leftover UPnP mapping remains,
  Helix offers a typed `REMOVE AMP FORWARD <port>` action that deletes that
  router mapping only. Minecraft Settings can still move the Helix server off
  that port.
- Homarr import names stay inside the picker and shortcut tiles instead of
  spilling across neighboring cells.
- Homarr import picker rows keep their height and scroll instead of stacking
  twenty-plus names on top of each other.
- Homarr import keeps http(s) icons, maps Homarr icon names onto the
  dashboard-icons set, and matches leftover shortcuts from the app name or
  link. Uploaded Homarr media files stay in Homarr.
- The Linux from-source installer asks yes/no questions on a terminal, offers
  another loopback port when 8080 is taken, and can install compiler packages
  or Rust without extra flags.
- Homarr import reads the current Homarr SQLite app catalog (http(s) hrefs from
  the container mounts) instead of telling you to export classic JSON by hand.
- README CI badge tracks the latest push to `main`, and CodeQL uses the repo
  query filter so local-path false positives stop reopening.
- The local package uses the distro nologin path for the `helix` account, and
  hook one-click preflight no longer demands APT/dpkg on Fedora-style hosts.
- Linux Mint Debian Edition maps Tailscale/Jellyfin repos to Debian instead of
  inventing an Ubuntu suite from Mint's VERSION_CODENAME.
- From-source `--install-deps` follows the usual `/etc/os-release` symlink, so
  Debian, Ubuntu, Fedora, and Arch are not treated as an unknown distro.
- Backup delete no longer hid behind a missing server-detail capability. Anyone
  with backup manage can trash a copy, undo it, or delete it forever.

### Added

- Host Terminal keeps Tab in the shell so bash can complete paths and commands,
  starts an interactive login shell with the account's real HOME/locale, and
  behaves closer to SSH in the browser: copy/paste, find in scrollback,
  clickable http(s) links, font zoom, and mouse reports for tools like htop.

- Globe is an optional world map of this host and country-level connections.
  It stays off the sidebar until you add it from Arrange or Settings. A Home
  widget is available too. Lines stay solid unless you turn on data motion.
  Pins are countries, not streets, and remote addresses never leave the host.
- Arrange (and Settings → Navigation) can hide or add the built-in pages, not
  just reorder them. Settings stays pinned. Reset pages restores the factory
  set, with Globe hidden.

- Native servers can change allocated memory after create. Helix rebinds the
  container (and Minecraft `-Xmx`) to the new limit.
- Overview PUBLIC INTERNET shows the public IP and game port, plus a reminder
  to port-forward that port. It does not run UPnP setup from that card.

- Homarr shortcuts land on the Home you are editing. Import skips URLs already
  on that Home and does not create a separate Homarr layout.
- Copy a Home widget, then paste it on the same layout or another Home. Settings
  can send a copy to a different Home without leaving the current one.
- Minecraft Settings can change the published game port. Helix rewrites
  `server.properties`, rebinds the container, and drops public access on the
  old port instead of leaving a stale router mapping.
- Native Minecraft list and detail TPS come from a short local `/tps` console
  sample. Paper-family software and some plugins report a number; Vanilla and
  most Fabric/Forge/Quilt servers still show —. Helix caches that sample and
  keeps the RCON client start/shutdown lines out of the persistent console.
- Copy all copies every widget on a Home; Paste onto another Home drops the
  whole set at once. Ctrl/Cmd+A selects all tiles while editing.
- Server marketplace can search Modrinth or CurseForge for plugins, Bukkit
  addons, mods, and (on mod loaders) CurseForge modpacks. Descriptions render
  markdown/HTML. Search cards show Installed after a successful add, and have
  an Install button that writes the JAR into plugins/ or mods/ without restarting.
- Native backups can cap how many copies to keep and how many days to keep
  them. Extra oldest copies move to trash. Delete forever is available from
  the active list or from trash after a confirm.
- Advanced, console, marketplace, and the server list explain jargon in place
  (loopback-only RCON, TPS, OOM killed, SHA-256, and similar).
- Server cards put Open next to Restart or Start. Imported AMP servers put
  Open AMP in that same row.

- `scripts/install-from-source.sh --port` / `--listen` for a loopback bind
  other than 8080 on a fresh config. `--yes` skips prompts. `install-local.sh
  --listen` matches that for new `/etc/helix/helix.toml` files only.
- `scripts/install-from-source.sh` builds the unsigned local `helixd` package
  on systemd Linux and installs it with sudo. Missing compiler tools print
  apt, dnf/yum, zypper, pacman, apk, or emerge commands; `--install-deps`
  installs those packages. The installer follows `/etc/os-release` through the
  usual distro symlink, accepts pkgconf, and requires GNU coreutils. Host and
  game controls still need `helix-privd`.

- Notifications bell for capacity warnings, dismissible Overview/Storage
  notices, a quiet Help link to the GitHub wiki, Valheim and Terraria native
  create, CurseForge catalog browsing/install without an owner API key, Forge /
  NeoForge / Quilt / Pufferfish as default Minecraft software, a Portainer hook,
  and Settings management for native Helix servers and recoverable trash.

- Security center, Docker/Portainer inventory, Homarr shortcut import, Home
  fullscreen, live Overview/Home graphs, a Helix-only server filter, and hook
  resource chips. V Rising create is click-to-install with runtime cleanup
  when the last server is removed. Minecraft versions are a published-release
  dropdown. The login and sidebar wordmark is the HELIX text only.

- Ten-crate Rust workspace with typed configuration, separate
  critical-state and replaceable-metrics SQLite domains, a versioned HTTP API,
  and a local administration CLI.
- Race-safe local owner enrollment, Argon2id login, bounded and revocable
  server-side sessions, capability checks, session-bound CSRF proofs, and
  append-only authentication audit events.
- Portable encrypted secret records using per-record data keys and a separately
  supplied installation master key.
- Real read-only CPU, memory, uptime, swap, storage, and cumulative network
  discovery.
- Compiled Preact dashboard with setup/login/session-expired flows, honest
  partial states, native progress meters, responsive layouts, and System,
  Midnight, OLED, and Light themes.
- Recovery, storage, security, architecture, API, packaging, and extension
  contracts.
- Hardened systemd, local bundle, checksum-verifying installer, rollback
  scaffolding, and pinned CI/security tooling. One scoped Ubuntu 24.04 systemd
  package lifecycle passed for commit `fdbbc0a`.
- GNU AGPL v3-or-later project licensing, matching Rust/npm metadata, a visible
  source link, and a checksummed license in release bundles.
- A game-hosting capacity contract separating game/player performance from
  Helix control-plane bounds, resource admission, and required load evidence.
- A lazy-loaded Games workspace with responsive card and compact views,
  filtering, instance detail and capability-driven tabs, plus a protected
  readiness API that keeps production controls locked until restore, broker,
  and native-execution gates are real.
- Installable UI Strands: pack a zip, drop it on the Strands page or paste an
  https zip URL, review the exact host calls, then Enable. Isolated pages and
  Home widgets can read bounded metrics, keep their own key/value store, and
  call allowlisted HTTPS APIs. There is no Helix store and no Wasm/native
  sidecar runtime.
- A full private-LAN dashboard with Overview, multi-layout Home, Storage,
  Network, Host, Terminal, Servers, Hooks, and Settings pages; reorderable
  navigation; and bounded color/theme controls.
- Native Docker-backed Minecraft creation and lifecycle for Paper, Purpur,
  Folia, Leaves, Fabric, and Vanilla, including a published-release version
  picker, persistent console archives, settings, files, performance,
  recoverable backups/removal, and custom server artwork.
- Drag-and-drop / chunked uploads in Storage and native server Files, plus
  dropping a custom server JAR during create.
- Compatibility-filtered Modrinth project artwork/search/details/install and a
  narrowly verified server-safe Fabric `.mrpack` creation path.
- Configured-root file browsing with pagination, drag-and-drop uploads,
  explicit edit/rename actions, recoverable trash, and quick/thorough
  cancellable largest-item analysis.
- Network evidence, exact Helix-owned UFW rules with Undo, and a separate
  confirmed SSH-safety flow for enabling an installed inactive UFW.
- Exact selected APT update jobs with list refresh, immediate candidate
  revalidation, holds/disk/no-removal/conffile/final-version guards, bounded
  logs, and no automatic reboot or rollback claim.
- Immediate and recurring host reboot scheduling, exact Helix start-after-boot
  policy control, Hooks for allowlisted services, and a separate authenticated
  non-root Linux PTY service.

### Changed

- Home full screen hides the top bar as well as the sidebar. Create-dialog
  toggles and Docker inventory copy sit inset from the window edge. Security
  defaults to host hardening, with Fail2ban, NTP, sysctl, and SSH facts plus
  written recommendations. Docker listing prefers a compact engine format and
  opens Portainer on 9443/9000 instead of a dead Open Docker link. Minecraft
  modpack cards clamp long titles. Imported and dedicated-game marks are
  geometric instead of noisy artwork.

- Migration rollback snapshots are content-addressed and tied to the exact
  source schema, target schema, and source SHA-256. Identical retries reuse the
  verified snapshot; changed sources get a new one; altered aliases fail closed.
- Startup reconciles legacy migration partials in bounded batches and validates
  recovery before session cleanup.
- Shutdown drains HTTP requests and detached blocking state/password work before
  writing the clean-shutdown marker.
- Unlabeled or invalid storage text is reported as an explicit omission instead
  of breaking the host overview.
- The 320-pixel dashboard now wraps long interface names and avoids document
  overflow on browsers with non-overlay scrollbars.
- Setup values now survive validation and recoverable request failures; a real
  already-claimed conflict still clears the one-time credentials.
- Dashboard transitions move keyboard focus deliberately, health changes use a
  polite live region, forced-colors focus remains visible, and route context
  follows the active section.
- Updated `toml` to 1.1.4 while retaining Rust 1.88 compatibility and removing
  the duplicate `winnow` 0.7 dependency.
- Theme and section-navigation state now rerender only their small controls;
  setup, login, refresh, and logout use native non-repeat disabled states.
- Frontend production builds now fail above 75 KiB initial gzip or 40 KiB
  initial JavaScript gzip, with focused parser and boundary tests.
- Information tooltips now use a viewport-aware document portal so cards and
  scroll containers cannot clip them. Marketplace artwork uses a constrained
  same-origin image proxy instead of direct arbitrary browser image requests.
- Home layouts now support templates, JSON export/import, drag handles,
  width/height, paged notes, quick note editing, widget colors, revisioned
  server persistence, and a retrying local fallback.

### Fixed

- Chunked uploads treat the 10-minute limit as time since the last chunk, so a
  large JAR on a slow LAN is not killed while data is still arriving.
- Paper, Folia, and Leaves version lists no longer advertise experimental
  Minecraft versions as Latest when create would install the current
  default/stable release.
- Dropping a file onto Storage or the custom-JAR field while an upload is busy
  no longer lets the browser navigate away from the dashboard.

### Security

- Runtime binding remains restricted to loopback even though local
  authentication is implemented. Remote exposure still requires the TLS/proxy,
  cookie, MFA, rate-limit, and independent-review boundary.
- Protected reads and mutations require both the session cookie and the current
  in-memory CSRF proof; cookie-only requests cannot recover a new proof.
- Login cleanup is bounded, oversized imported session sets converge through a
  generic retryable response, and final eviction/rehash/session/audit changes
  commit atomically.
- Authentication/session audit retention protects the newest 1,024 events,
  applies a 90-day window beyond that floor, targets at most 50,000 rows, and
  prunes at most 256 rows per transaction.
- API responses use restrictive browser headers and separate unknown API routes
  from the frontend fallback.
- Critical state uses durable SQLite settings, verified online snapshots,
  restrictive Unix-mode policy, and explicit unclean-shutdown checks.
- Terminal access requires the current Helix password for each 30-second
  single-use session-bound HttpOnly ticket. Exact Origin/subprotocol checks,
  a distinct socket group, Linux peer-UID verification, non-root execution,
  bounded frames/sessions, and lifecycle-only auditing protect the bridge.

### Known limitations

- Helix is not ready for production deployment or public-network exposure.
- Production master-key delivery/rotation/recovery, broader audit-event coverage,
  tamper evidence, off-host forwarding, complete clean-host restore, signed
  self-update, historical metrics, a third-party Strand runtime, and additional
  game lifecycles are not complete.
- A scoped Ubuntu 24.04 install/rollback/uninstall lifecycle has passed, but
  clean-VM, cross-version upgrade, complete permission/fault/recovery, signing,
  and reference-performance matrices remain open.
