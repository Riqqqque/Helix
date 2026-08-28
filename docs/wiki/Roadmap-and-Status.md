# Roadmap and Status

Helix keeps plans and evidence separate:

- [`PROGRESS.md`](https://github.com/Riqqqque/Helix/blob/main/PROGRESS.md) records
  implemented behavior and its validation level.
- [`NEXT.md`](https://github.com/Riqqqque/Helix/blob/main/NEXT.md) lists the next
  concrete validation work.
- [`ROADMAP.md`](https://github.com/Riqqqque/Helix/blob/main/ROADMAP.md) describes
  dependency order, not promises or dates.

The current private alpha includes the authenticated dashboard, modular Home,
typed Linux broker, configured-root storage tools, native Docker-backed
Minecraft manager, separate AMP bridge, persistent console history, recoverable
backups, safe host controls, network/UFW inventory and narrow owned-rule
management, guarded selected-package updates, Hooks, an optional non-root host
terminal, and a compatibility-aware Modrinth marketplace. A narrow “Start with a modpack” path creates
declared-hash-verified server-safe subsets from listed stable server-capable
Fabric `.mrpack` releases.

Public release remains blocked on supported-host lifecycle matrices,
independent security review, recovery and fault drills, live disposable UFW and
reboot validation, signed artifacts, safe update design, accessibility/mobile
review, and real Minecraft version matrices.

Broad/unattended Package Apply, package rollback, and signed self-update are
unavailable. Exact selected APT candidates can be applied after strict preflight
and explicit confirmation. Public exposure is not supported. Modpack create is a server-safe subset from
Modrinth or public CurseForge catalogs, not a full client copy. Unknown loaders
and every upstream pack remain unclaimed.

Owners can install UI-only Strands from a zip or https zip URL after reviewing
the exact host calls. Portable Wasm, native sidecars, signatures, and a
Helix-operated store are still not a runtime. Modular Home widgets are built
into Helix; a Strand can also be pinned there when it declares `helix:ui.widget`.
