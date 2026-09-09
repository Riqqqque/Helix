# Valheim verification — September 9, 2026

## Checked builds

- `npm run check`: lint, 343 frontend tests, three asset tests, TypeScript and production build passed for the publishable changes.
- `docker build --target linux-test`: Rust 1.88 release binaries, formatting, Clippy with warnings denied, 519 tests including ignored recovery tests, and the real HTTP/broker integration smoke passed.
- `python3 -m unittest discover -s crates/helix-privd/valheim -p 'test_*.py' -v`: ten Linux tests passed, including the shipped launcher's save-on-stop behavior.
- `node scripts/test-valheim-browser.mjs`: desktop and 390px mobile checks passed. These use the real components with isolated HTTP fixtures: creation options, save/reload, conflict handling, stopped-server locks, package preview, install jobs, update discovery, disable/remove and layout overflow. No page exceptions were reported.

The browser test needs Playwright and a local Vite server. `HELIX_PLAYWRIGHT` can point to an installed Playwright module; `HELIX_BROWSER_URL` overrides the default `http://127.0.0.1:5176/e2e/valheim.html`. `HELIX_BROWSER_OUTPUT` selects the screenshot directory. Test fixtures do not create production servers.

## Actual Linux game checks

A disposable container used a numeric non-root user, dropped capabilities, no Docker socket, two CPUs and a 5 GiB memory limit. Only its own data directory was mounted. Existing game servers were not restarted.

Verified with Valheim **1.0.7**, Thunderstore BepInEx pack **5.4.2350**, and Jötunn **2.29.2**:

- Steam install, vanilla startup, real readiness, world generation, graceful stop and reload.
- BepInEx and Jötunn load; Steam-only startup and PlayFab crossplay registration with a join code.
- Mod disable/re-enable, dependency removal protection, removal/reinstallation and catalog update checks.
- Steam file validation/repair without changing world or configuration hashes.
- Full archive integrity check, extraction comparison, and startup/save of the restored world.

The test caught and fixed a native networking-library crash when the restricted numeric UID had no passwd entry. The runtime supplies a container-local NSS identity; it does not create a host account. Tests also caught stale Cargo artifacts when switching source snapshots in a shared Docker build cache. Changed build layers now refresh source timestamps before compiling.

## Limits

This is not a client gameplay, ten-player load, arbitrary-mod compatibility or console-device join certification. Crossplay registration was verified; a real external player's connection was not. Headless Unity emits graphics warnings even when the server starts successfully.

The UI tests and real runtime/API tests are separate. They do not claim an authenticated production-browser creation test. Installing new broker controls requires an administrator to update the host broker along with the dashboard. Keep the existing deployment until that step is complete.

See [Valheim setup and recovery](VALHEIM.md). Backups protect files, but no manager can guarantee that third-party mods or a newer game build will preserve gameplay compatibility.
