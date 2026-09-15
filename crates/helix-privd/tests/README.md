# Shutdown fixtures

On Linux with Python 3 and a C compiler, run:

```sh
python3 crates/helix-privd/tests/runtime_shutdown.py
```

This exercises the shipped Valheim and Terraria launchers during startup and
after readiness, using disposable processes that write a save marker on exit.

From the repository root, test the V Rising Wine console path with:

```sh
docker build -t helix-vrising-runtime:2 crates/helix-privd/vrising
docker build -f crates/helix-privd/tests/Dockerfile.vrising-shutdown -t helix-vrising-shutdown-test crates/helix-privd
docker run --rm --network none --memory 1g --cpus 1 helix-vrising-shutdown-test
```

The fixture runs the actual launcher and shutdown helper with a small Windows
console executable instead of the game. It never mounts a real server directory.
These tests verify signal delivery and launcher behavior, not game-engine or
mod-specific world persistence.
