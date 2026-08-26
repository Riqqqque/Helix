# How Helix Works

Helix is a small local-first control plane for a Linux server. The compiled web
dashboard talks to one unprivileged `helixd` process. That daemon authenticates
the user, reads bounded host information, and keeps important local state in
SQLite. `helixctl` provides local setup, diagnosis, snapshots, and developer
tools.

Future game servers and ordinary services run as independent systemd units.
Helix controls them but is not their parent process, so a dashboard update or
crash should not stop a running game.

Work is separated by risk:

- `helixd`: lightweight API and orchestration;
- `helix-privd`: future socket-activated typed privileged operations;
- `helix-worker`: future one-shot heavy backup, restore, and verification
  jobs;
- `helix-strandd`: future optional sandbox host for third-party Strands;
- systemd: independent games and services.

The appeal is a fast interface that stays small when optional features are
unused, keeps expert controls visible, treats recovery as normal product
behavior, and does not make game uptime depend on the panel.

The current alpha has real authentication, durable state, host monitoring,
recovery checks, dashboard, CLI, package lifecycle evidence, and preview Strand
tooling. Game management, remote access, restore, and Strand execution are not
implemented.

Read the
[full walkthrough](https://github.com/Riqqqque/Helix/blob/main/docs/HOW-HELIX-WORKS.md)
for the request flow, data model, future components, and exact boundaries.
