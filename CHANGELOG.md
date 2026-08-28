# Changelog

This file records user-visible and operator-visible changes. Helix has no
binary or package release yet.

## Unreleased

### Fixed

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
- The sidebar HELIX wordmark is centered in the top-left column.

### Added

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
