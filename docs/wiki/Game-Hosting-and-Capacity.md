# Game Hosting and Capacity

Game management is planned, not implemented. Helix cannot honestly claim a
player limit yet.

Helix will sit outside the game's tick loop and player network path. Each game
server will remain an independent systemd unit and cgroup. The game, server
build, plugins or mods, world, configuration, and hardware determine actual
player capacity.

Helix is responsible for keeping management overhead out of the way:

- no hosting timer, poll, worker, or database writer while hosting is disabled;
- bounded cursor pages for instances, players, logs, and console data;
- detail fetched only while a view is open and long lists rendered in a window;
- coalesced reconnectable events instead of one polling loop per player or card;
- bounded queues, backpressure, and fair job concurrency;
- operator-approved resource envelopes and host headroom rather than unsafe
  automatic overcommit;
- game-specific capacity claims only after real versioned lifecycle and load
  tests.

A synthetic 10,000-player fixture can prove that the Helix API and UI stay
bounded. It cannot prove that a real game supports 10,000 players.

The full engineering and release contract is in
[`docs/GAME-HOSTING-CAPACITY.md`](https://github.com/Riqqqque/Helix/blob/main/docs/GAME-HOSTING-CAPACITY.md).
