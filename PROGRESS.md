# Helix Progress

Last updated: 2026-08-29

This is the implementation ledger. [ROADMAP.md](ROADMAP.md) describes intended
ordering; it is not evidence that a feature works.

Status vocabulary: **NOT STARTED**, **DESIGNING**, **IMPLEMENTING**,
**IMPLEMENTED — UNVALIDATED**, **TESTED**, **BLOCKED**, **COMPLETE**.

## Overall status

Helix is a **private-LAN 1.0 release**. The authenticated dashboard, typed Linux broker,
native Minecraft manager, optional AMP bridge, Hooks, multi-layout Home,
storage tools, selected package updates, host controls, optional
unprivileged terminal, and installable UI-only Strands are implemented. Portable tests and focused mock/pure
Linux-boundary tests pass in the current workspace.

This is not a public-internet release claim. Clean supported-host matrices, destructive
fault injection, independent security review, independently signed artifacts,
recovery drills, broad game-version coverage, and public-network
review remain open. Public-internet release status is **BLOCKED**.

## Current implementation

| Area | Status | Current evidence and limits |
| --- | --- | --- |
| Repository, license, and build policy | TESTED | The Rust workspace and Preact frontend use locked dependencies, formatting/lint/test/build gates, secret scanning policy, and `AGPL-3.0-or-later`. Passing source checks is not a signed release. |
| Authentication and preferences | TESTED | Race-safe owner enrollment, Argon2id login, revocable bounded sessions, session-bound CSRF, capability checks, username/password changes, and revision-guarded dashboard preferences have focused state/API/frontend tests. MFA, production remote-access review, and independent authentication review remain open. |
| Critical state and recovery foundation | TESTED | SQLite integrity checks, migrations, exclusive writer lease, unclean-shutdown handling, verified state snapshots, and data-preserving package skeletons exist. Full broker/native-data restore drills, power-loss matrices, independent key recovery, and signed upgrade rollback remain open. |
| Dashboard shell | TESTED | Overview, Home, Storage, Network, Host, Security, Terminal, Servers, Hooks, and Settings are full responsive pages. Empty URL fragments open Home. Reorderable navigation, System, Midnight, OLED, and Light themes, bounded color controls, keyboard/focus states, reduced motion, lazy feature chunks, and initial asset budgets are enforced. Formal screen-reader and representative-device review remain open. |
| Modular Home | TESTED | Users can create, rename, switch, duplicate, export, import, and remove bounded Home layouts, including a full-screen Home mode. Clock, host, graphs, servers, storage, docker, weather, paged-note, and HTTP(S) shortcut widgets support drag reordering, width/height, title, color, and type-specific settings. Homarr import reads classic JSON or a SQLite app catalog (`app`/`apps` with `name` and `href`) from Homarr container mounts; only http(s) shortcuts are offered, and URLs already on that Home are skipped. Preferences use bounded revisioned server state with a local fallback while syncing. This is a built-in dashboard system. An enabled Strand that declared `helix:ui.widget` can also be pinned as a widget; that still uses the sandboxed Strand iframe, not a native plugin. |
| Read-only host visibility | TESTED | CPU, memory, swap, uptime, disks, mounts, interfaces, routes, services, processes, and bounded listeners come from real adapters with explicit degraded/unavailable states. Reference Linux performance and platform matrices remain open. |
| `helix-privd` boundary | IMPLEMENTED — UNVALIDATED | A Unix-socket broker accepts a closed typed protocol for configured storage, host, network, native server, marketplace, and AMP operations. It has no caller-supplied shell command RPC. Pure/mock tests cover validation and failure paths; the complete clean-host peer-credential, systemd-sandbox, race, and fault matrix remains open. |
| Storage and files | IMPLEMENTED — UNVALIDATED | The dashboard browses configured roots with bounded pagination and type/size columns, and supports drag-and-drop/chunked uploads up to 256 MiB, 4 MiB validated UTF-8 editing, file/folder creation, explicit rename, and recoverable trash. Quick and user-triggered thorough largest-file/folder analysis distinguish filesystem coverage from bounded top-result retention and allow confirmed trash directly from results. Descriptor-relative Linux traversal and cancellation have focused coverage. Broad mount-replacement, low-disk, permission, and live multi-terabyte scans remain target-host gates. |
| Native Minecraft manager | IMPLEMENTED — UNVALIDATED | Docker-backed creation and lifecycle paths exist for Paper, Purpur, Folia, Leaves, Fabric, Forge (installer, Minecraft 1.17+), NeoForge, Quilt, Pufferfish, and Vanilla. Creation loads published Minecraft releases (Latest plus an exact picker). Custom JARs can be dropped into Helix's private import root. Native detail, start/stop/restart/confirmed SIGKILL when stop hangs, latest-compatible-build update, settings, files, logs, performance, console, backup, restore, recoverable removal, uploaded/preset server artwork, and a post-create start-on-boot toggle (`unless-stopped` / `no`) are wired through typed APIs. Real supported-Ubuntu lifecycle/version matrices remain incomplete; arbitrary historical Paper/Purpur build IDs and broad player-capacity claims are not implemented. Kill is native Docker only; AMP instances stay under AMP. |
| Native V Rising manager | IMPLEMENTED — UNVALIDATED | Helix builds `helix-vrising-runtime:1` locally (Debian, Wine64, Xvfb, SteamCMD), runs Steam app 1829350 as Windows, stores game files in the instance directory, and removes the image when the last active V Rising server is uninstalled. The create UI is click-to-install without a Wine checkbox. Public UPnP is not offered. Player counts are not queried. The first SteamCMD download and a live-host create/start matrix remain open. |
| Native Valheim manager | IMPLEMENTED — UNVALIDATED | Helix builds `helix-valheim-runtime:1` locally, installs Steam app 896660, and occupies three consecutive UDP ports. Public UPnP is not offered. Optional BepInEx is a zip plus plugin files in the instance data directory, then a restart. The first SteamCMD download and a live-host create/start matrix remain open. |
| Native Terraria manager | IMPLEMENTED — UNVALIDATED | Helix builds `helix-terraria-runtime:1` locally. Vanilla uses the publisher dedicated zip; tModLoader uses Steam app 1281930. `.tmod` files in `/data/mods` apply after restart. Public UPnP is not the default pool behavior. Live-host create/start remains open. |
| Servers dashboard module | TESTED | Owner setup asks whether to include the Servers page (default yes). Existing owners stay enabled. Settings can hide or restore the page later without stopping running game containers. Navigation order still stores every section. |
| AMP bridge | IMPLEMENTED — UNVALIDATED | AMP is an optional separate manager reached through a loopback-only credentialed integration. Helix can inventory AMP instances and issue bounded supported actions without adopting their files or identity as native instances. AMP outages and ambiguous responses fail closed. Broader live AMP-version testing remains open. |
| Persistent console history | IMPLEMENTED — UNVALIDATED | Native console output is captured to bounded protected rotating files independent of the browser. History pages return at most 500 entries with retained boot/session metadata where available; the legacy tail response remains stable. Retention and restart logic have focused tests, while long-running disk/failure testing remains open. |
| Native backups | IMPLEMENTED — UNVALIDATED | Backup creation/restore exists. Delete moves exact native backup artifacts into protected recoverable trash with opaque IDs and expiry metadata; Undo restores only matching records. Full interrupted-backup, disk-full, restore-to-clean-host, and retention-purge matrices remain open. |
| Settings restart metadata | TESTED | Server settings expose which fields require restart and save responses report pending restart state instead of implying immediate application. Settings writes use revisions and preserve a rollback copy. Game-version-specific behavior still needs broad live coverage. |
| Modrinth marketplace | IMPLEMENTED — UNVALIDATED | Search, project detail, compatible-version selection, and background install jobs exist for Paper/Purpur/Leaves plugins, Folia-compatible plugins, and Fabric/Forge/NeoForge/Quilt server-side mods. Loader/version/environment checks prevent mixing plugins and mods. CurseForge uses `api.curseforge.com` with an owner-supplied key from Settings → Catalogs. Real upstream outage, artifact, dependency, and lifecycle matrices remain open. Vanilla does not have a plugin/mod install path. |
| Modpack creation | IMPLEMENTED — UNVALIDATED | “Start with a modpack” searches Modrinth, and CurseForge when an owner API key is saved. Helix downloads a selected pack, pins Fabric/Forge/NeoForge/Quilt, and starts an isolated server. Modrinth `.mrpack` still verifies declared hashes and excludes client-only files. CurseForge uses `manifest.json` plus forgecdn files through the official API and does not claim SHA-512 parity. Linux extraction/resolver/Docker lifecycle and real upstream/pack matrices remain open. Output is a server-safe subset, not a full client copy. |
| Host integration and start on boot | IMPLEMENTED — UNVALIDATED | Status reports Docker and broker service state, exact dashboard/gateway container identities and restart policies, and bounded Helix-only resources/errors. The toggle changes only those exact containers and does not stop/start them or alter Docker itself. Docker restart metadata survives a daemon/host restart, but later container recreation may reapply Compose policy. |
| Host reboot | IMPLEMENTED — UNVALIDATED | Immediate requests use a 10–300 second cancellable delay. Recurring daily/weekday schedules use one exact local time and verified host timezone. Both require exact hostname, disruption acknowledgement, and active-player/running-job preflight; one-shot work uses an opaque systemd transient timer. Tests use pure/mock runners and never reboot a host. A disposable live Linux validation is still required. |
| Network inventory | IMPLEMENTED — UNVALIDATED | Linux TCP/UDP listeners, best-effort process ownership, Docker publications/bind addresses, private IPv4, bounded same-router UPnP discovery, WAN-address/CGNAT classification, native/AMP game ports, exact Helix-owned router mappings, UFW state, and externally unverified reachability are separate evidence. A listener, UFW rule, or confirmed router mapping is never labeled as proof of outside access. |
| Game port pools and forwarding | IMPLEMENTED — UNVALIDATED | Minecraft can allocate from bounded ordered ranges and priority ports while creation is serialized, skipping assigned/bound ports. Opt-in public setup refuses existing router mappings, verifies one exact TCP UPnP mapping, journals ownership, and adds UFW only when already active. Controlled-router mutation, CGNAT, reboot, and multi-router matrices remain open. |
| UFW rule management | IMPLEMENTED — UNVALIDATED | Named TCP/UDP single-port or bounded-range allow rules use exact opaque Helix comments, durable ownership records, a global mutation lock, before/after verification, safe delete, and bounded Undo. A separate exact-phrase activation flow first proves the selected SSH TCP port is listening, stages a durable SSH safety rule, enables UFW, verifies both, and attempts to restore inactive state on failure. Helix never resets UFW or changes defaults. These mutations have mock/pure coverage but have not completed a disposable live-UFW matrix. |
| System packages | IMPLEMENTED — UNVALIDATED | dpkg/APT inventory reports installed/candidate versions, sizes, source/category/description, held/security/restart hints, cache timestamp, and preview state. Explicit refresh and exact selected-candidate apply jobs serialize mutations, revalidate versions/holds/disk/no-removal preview, preserve current conffiles, verify final versions, retain bounded logs, and never reboot automatically. APT HTTPS runs as root inside the broker sandbox so `_apt` seteuid is not required. There is deliberately no package rollback claim. Disposable interruption/conffile/dpkg-recovery matrices remain open. |
| Helix self-update | IMPLEMENTED — UNVALIDATED | Host → Linux updates checks GitHub for a newer `vMAJOR.MINOR.PATCH` release and can apply a SHA-256-pinned source archive. The job rebuilds only dashboard/gateway, replaces helix-privd and helix-terminald, health-checks, and restores those on failure. `git pull` is not used. Game containers, AMP, and Plex stay running. Independently signed keys, interruption matrices, and a live apply drill remain open. |
| Hooks | IMPLEMENTED — UNVALIDATED | The broker inventories exact allowlisted Plex, Tailscale, Pterodactyl Wings, and Jellyfin systemd units plus the AMP API adapter, Docker Engine containers, and a detected Portainer UI. Cgroup memory/CPU is used when systemd exposes it; Docker totals use engine stats when `docker stats` succeeds. The UI provides verified supported lifecycle/start-after-boot actions, local panel links, and guided official setup for absent services. It does not run remote install scripts, invent credentials, or claim full upstream API parity. |
| Optional host terminal | IMPLEMENTED — UNVALIDATED | A lazy xterm frontend opens a real PTY through a separate non-root Linux service. Every connection requires the current Helix password, a 30-second single-use session-bound HttpOnly ticket, exact Origin/subprotocol checks, a distinct socket group, and Linux `SO_PEERCRED` matching the pinned dashboard UID. Commands/output are not audited or retained; disconnect kills the PTY. Protocol/unit tests, a real Linux PTY/resize/exit smoke, exact accepted-peer and rejected-peer checks, and a no-I/O-in-daemon-logs check pass. Clean-host service, authenticated browser, broader hostile-peer, sudo, and fault matrices remain open. |
| Tailscale compatibility | IMPLEMENTED — UNVALIDATED | Container configuration can expose an explicitly constrained second private gateway suitable for an existing Tailscale route, while Hooks can detect/control an already installed exact service. Helix does not install, authenticate, or reconfigure Tailscale. Public-network exposure remains unsupported. |
| Installable UI Strands | TESTED | `helixctl strand new/check/pack/inspect` plus dashboard install from zip or https URL. UI-only packages run in a sandboxed iframe and may call bounded metrics, namespaced KV, and origin-allowlisted HTTPS after Enable. Portable Wasm, native sidecars, signatures, and a Helix-operated store are not a runtime. |

## 2026-08-27 existing-host deployment evidence

A private Ubuntu host was upgraded in place from state schema 6 to 7 after an
integrity-checked rollback snapshot was created. The dashboard and gateway
started healthy with zero container restarts, `helixctl doctor --full` passed,
and the complete set of unrelated Docker workloads remained running across the
Helix-only container replacement.

The installed broker and terminal sockets had the expected distinct groups and
`0660` modes. The terminal service ran as the configured non-root Linux user,
accepted only the pinned dashboard peer UID, completed a real PTY user,
working-directory, resize, output, and exit smoke, rejected a wrong peer UID,
and did not place terminal input or output in journald. No reboot, firewall
mutation, package application, native-server deletion, or live data deletion
was performed during this pass.

This is useful target-host evidence, not a clean-install or public-release
matrix. The broader destructive fault, recovery, hostile-peer, authenticated
browser-terminal, and supported-host gates below remain open.

## 2026-08-28 in-place upgrade

The same private Ubuntu host was upgraded in place to git `cb37f20` (native
Kill) plus a dashboard image that forces packaged web assets to mode `0444`.
The first new image refused to start because `favicon-32.png` arrived with
group-write bits. After the mode fix, dashboard and gateway were healthy,
`helixctl doctor --full` passed on schema 7, the LAN gateway returned HTTP 204,
and only the two Helix web containers changed identity. The native Minecraft
container, AMP instances, and other Docker workloads kept their previous IDs
and stayed running. Helix-privd and helix-terminald were replaced from the
same build. No reboot, firewall mutation, marketplace install, or live game
stop was performed.

## Explicit unsupported states

- Forge/NeoForge/Quilt/Pufferfish native creation is implemented and unvalidated.
  Live-host installer, unix_args launch, and publisher-CI matrices remain open.
- Modpack install is a server-safe subset for Modrinth `.mrpack` and CurseForge
  `manifest.json` packs. It is not a full client copy and has not finished a
  broad pack matrix. Helix does not ship a CurseForge API secret; the owner
  supplies one in Settings → Catalogs.
- Broad unattended upgrades, dependency additions/removals, and package rollback
  are not supported. Only explicitly selected exact candidates that pass the
  documented APT preflight are accepted.
- Helix self-update is not supported.
- External port reachability is not tested or guaranteed.
- UFW inactive/unavailable is never presented as “port open.”
- Tailscale installation, tailnet authentication, and route configuration are
  not performed by Helix; only an existing service lifecycle is manageable.
- V Rising native creation uses a Helix-owned Wine runtime. It is implemented
  and unvalidated; do not treat it as publisher-supported or live-host proven.
- AMP is not the Helix native runtime.
- Public internet exposure and a supported public package are not approved.

## Current validation gates

The checked workspace runs these core gates:

```text
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features

cd frontend
npm run check
```

Linux-only broker code also has pure/mock tests that never execute real reboot,
firewall, package, or destructive workload mutations. Those tests prove input,
ownership, concurrency, verification, and failure behavior; they do not replace
a disposable live-host matrix.

## Public-release gate

**BLOCKED.** Before a public recommendation, Helix still needs:

- clean supported-Ubuntu install/upgrade/rollback/uninstall matrices;
- independent authentication and broker security review;
- production master-key delivery, rotation, and independent recovery;
- complete filesystem race, disk-full, interruption, and restore drills;
- disposable live reboot and UFW validation;
- disposable selected-package interruption, conffile, and dpkg recovery tests;
- signed, digest-pinned Helix releases with staged health rollback;
- real lifecycle matrices across every advertised Minecraft software/version;
- accessibility and representative mobile review; and
- reference performance and long-running retention evidence.

No signed package, automatic updater, public-network recommendation, or broad
Minecraft/modpack support claim is authorized by the current evidence.
