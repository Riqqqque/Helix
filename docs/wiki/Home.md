# Helix Wiki

Helix is a local-first Linux server dashboard built around a small unprivileged
daemon, recoverable local state, and independently managed workloads.

> Helix is currently an alpha engineering preview. It is loopback-only and is
> not supported for production servers or irreplaceable data.

## Start here

- [Getting Started](https://github.com/Riqqqque/Helix/wiki/Getting-Started)
- [Architecture](https://github.com/Riqqqque/Helix/wiki/Architecture)
- [Security and Recovery](https://github.com/Riqqqque/Helix/wiki/Security-and-Recovery)
- [Development and Testing](https://github.com/Riqqqque/Helix/wiki/Development-and-Testing)
- [Roadmap and Status](https://github.com/Riqqqque/Helix/wiki/Roadmap-and-Status)

## Current working surface

- one-time local owner setup and password authentication;
- revocable, bounded sessions with session-bound CSRF protection;
- real CPU, memory, swap, storage, network, uptime, OS, architecture, and kernel
  overview data;
- durable critical SQLite state and a separate replaceable metrics database;
- local `status`, `doctor`, `ready`, `setup-token`, and verified `backup-state`
  CLI operations;
- responsive System, Midnight, OLED, and Light dashboard themes;
- Linux package install, rollback, and data-preserving uninstall tooling that is
  covered by one scoped Ubuntu 24.04 systemd lifecycle but not supported for use.

Host mutation, remote access, games, services, files, restore, Vault, Genome,
Strands, and automation remain future work. The dashboard does not show fake
successful controls for those unfinished capabilities.

The authoritative evidence is always
[`PROGRESS.md`](https://github.com/Riqqqque/Helix/blob/main/PROGRESS.md).
