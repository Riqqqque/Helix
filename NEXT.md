# Next Helix Work

Last updated: 2026-08-29

## Current resumption point

Helix is a working private-LAN 1.0 release with a native Docker-backed Minecraft manager,
an optional separate AMP bridge, persistent console history, recoverable
backups, multi-layout Home, Hooks, storage analysis, guarded selected-package
updates, host controls, and an optional non-root terminal.

The next work is validation and hardening, not adding optimistic buttons to
unfinished backends.

## Priority order

1. **Run the full Linux gate on a disposable target.** Verify the exact broker
   and terminal service/socket identities, distinct groups, `SO_PEERCRED`,
   owners/modes, writable roots, sandbox exceptions, dashboard/gateway
   containers, and no unrelated workload changes.
2. **Exercise each advertised native Minecraft path.** For Paper, Purpur,
   Folia, Fabric, and Vanilla, cover install, start, query, console, settings,
   restart, stop, update, backup, restore, crash recovery, and removal behavior
   on pinned current versions.
3. **Prove console retention over time.** Keep the dashboard closed through
   multiple server boots, rotate retained segments, paginate older entries, and
   inject disk/read-only/interruption failures without losing current state.
4. **Complete native backup failure matrices.** Test interrupted creation,
   disk-full behavior, exact trash/Undo expiry, corrupt archives, restore into a
   clean fixture, and preservation of the last known-good copy.
5. **Validate storage on disposable trees and mounts.** Cover symlink and mount
   replacement races, deep/wide trees, cancellation, permission loss, low disk,
   and large-file/folder ordering. Do not point stress tests at irreplaceable
   media libraries.
6. **Validate network inventory without changing policy.** Compare listener,
   Docker publication, UFW, and game-port evidence on the target. Keep outside
   reachability labeled unknown unless an actual external probe is added.
7. **Run UFW mutation tests only on a disposable rule set.** Verify exact
   Helix-owned create/delete/Undo, crash-pending reconciliation, inactive and
   unavailable behavior, the separate SSH-safety activation/failed-activation
   recovery flow, and proof that unrelated rules/defaults never change.
8. **Validate host integration controls.** Confirm start-on-boot changes only
   the exact dashboard/gateway restart policies. Schedule and cancel reboot
   timers with a mock or disposable host first; never let an automated test
   reboot a live workload host.
9. **Validate selected package jobs on disposable Ubuntu fixtures.** Cover stale
   candidates, holds, locks, low disk, unexpected additions/removals, conffile
   preservation, maintainer-script failure, interruption, partial dpkg state,
   bounded logs, final-version proof, and the no-auto-reboot contract. Keep the
   explicit no-rollback claim.
10. **Drill Helix GitHub updates on a disposable host.** Cover a newer tagged
    archive, SHA-256 mismatch rejection, missing Compose project, disk-full,
    health-check failure rollback, and that unrelated container IDs stay the
    same. Independently signed Helix keys remain a public-internet gate. Do not
    implement `git pull` as an updater.
11. **Exercise Modrinth content and Fabric `.mrpack` creation.** Test real
    Paper/Purpur/Folia plugin and Fabric server-mod installs, then run pinned
    stable server-capable Fabric packs through search, preview, archive/hash
    checks, server-safe exclusions, startup, backup, restart, and rollback.
    Cover upstream errors, wrong loaders/versions, duplicates, disk pressure,
    partial downloads, and client-only content without claiming full-pack
    parity.
12. **Finish release gates.** Independent security review, master-key delivery
    and recovery, public-boundary review, accessibility, mobile, performance,
     and clean install/upgrade/rollback/uninstall matrices remain required.
13. **Exercise the optional terminal boundary.** Cover wrong/stale/replayed
    tickets, cross-origin upgrades, wrong peer UID/group, concurrent limits,
    disconnect/kill, resize/UTF-8/full-screen apps, sudo policy, service restart,
    and proof that commands/output never enter logs or audit.

## Private network and Tailscale

Keep the default source-development bind on loopback. A private container
deployment may use an explicitly configured LAN gateway and a separately
constrained secondary entry point suitable for an already configured Tailscale
route.

Validation must confirm exact Host/Origin/client-CIDR handling on every entry
point. Helix must not install, enable, authenticate, or reconfigure Tailscale.
Do not treat the whole Tailscale carrier-grade NAT range as trusted merely
because one node uses Tailscale.

Public internet exposure remains out of scope for this private-LAN release.

## Minecraft expansion rules

- Paper, Purpur, Folia, Fabric, and Vanilla are the current native choices.
- Forge and NeoForge stay visibly unavailable until their current Java,
  installer, mappings, artifact, license, lifecycle, and update behavior are
  implemented and tested from official sources.
- Do not describe a catalog explanation as support.
- The current modpack path is intentionally narrow: listed stable
  server-capable Fabric releases from Modrinth with one unambiguous `.mrpack`,
  pinned Minecraft/Fabric Loader, verified declared hashes, strict archive
  bounds, and server-safe exclusion of optional/client-only files. It is not
  byte-for-byte full-pack parity.
- Do not expand that evidence into broad modpack support. Forge, NeoForge,
  Quilt, unknown loaders, client parity, update/dependency reconciliation, and
  real pack matrices need their own implementation and tests.
- CurseForge integration needs an explicit terms-compliant API and artifact
  plan. Do not scrape or silently mirror it.
- Folia content must remain Folia-compatible; Paper fallback is not assumed.
- Vanilla does not receive a fake plugin/mod marketplace.

## AMP boundary

AMP remains separate software with its own instances, files, credentials, and
lifecycle rules. Helix may show and invoke supported AMP operations through the
loopback bridge, but it must not:

- move or rewrite an AMP instance as part of discovery;
- identify an AMP instance as Helix native;
- assume native backup/settings/marketplace semantics apply to AMP; or
- report success when AMP is stopped, unreachable, unauthorized, or ambiguous.

## Do not do next

- Do not publish a public release or mark the project production-ready.
- Do not expose setup/login/API routes directly to the public internet.
- Do not add a general privileged shell or caller-selected root command.
- Do not broaden selected APT Apply into unattended upgrade, removal, dependency
  installation, rollback, or Helix self-update claims.
- Do not claim a listener, Docker mapping, or UFW rule proves outside access.
- Do not reset UFW, change its defaults, or bypass the confirmed SSH-safety
  activation flow.
- Do not install Tailscale automatically.
- Do not claim Forge, NeoForge, Quilt, CurseForge, or broad/full-parity modpacks
  are supported.
- Do not run destructive storage, firewall, reboot, or package tests against a
  host with live users or irreplaceable data.
- Do not commit tokens, passwords, private addresses, hostnames, deployment
  roots, logs, databases, or generated build output.

## When a validation pass finishes

Record:

- exact revision and host conditions;
- commands and test results;
- which operations were real, mocked, or skipped;
- preserved workloads and rollback state;
- failures and unverified edge cases; and
- the narrow claim the evidence supports.

Then update [PROGRESS.md](PROGRESS.md) without converting a focused pass into a
general platform-support claim.
