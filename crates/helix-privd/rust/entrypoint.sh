#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/logs /data/steamcmd

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

# Steam query needs the client library where the game looks for it.
if [ -f /data/steamcmd/linux64/steamclient.so ]; then
  mkdir -p /data/.steam/sdk64
  cp -f /data/steamcmd/linux64/steamclient.so /data/.steam/sdk64/steamclient.so
fi

echo "Helix: updating Rust dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 258550 validate \
  +quit

if [ ! -f /data/server/RustDedicated ]; then
  echo "Helix: RustDedicated was not installed by SteamCMD" >&2
  exit 1
fi
chmod +x /data/server/RustDedicated 2>/dev/null || true

SEED=${HELIX_WORLD_SEED:-}
if [ -z "$SEED" ]; then
  SEED=$(od -An -N4 -tu4 /dev/urandom | tr -d ' ')
fi

cd /data/server

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

./RustDedicated \
  -batchmode -nographics \
  -logfile /data/logs/rust.log \
  +server.port "${HELIX_GAME_PORT:-28015}" \
  +server.queryport "${HELIX_QUERY_PORT:-28016}" \
  +rcon.port "${HELIX_AUX_PORT:-28017}" \
  +rcon.password "${HELIX_RCON_PASSWORD:?Helix did not pass an RCON password}" \
  +rcon.web 1 \
  +server.hostname "${HELIX_SERVER_NAME:-Helix Rust}" \
  +server.maxplayers "${HELIX_MAX_PLAYERS:-50}" \
  +server.identity helix \
  +server.level "Procedural Map" \
  +server.worldsize "${HELIX_WORLD_SIZE:-3500}" \
  +server.seed "$SEED" \
  +server.saveinterval 300 &
SERVER_PID=$!

i=0
while [ "$i" -lt 180 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Rust exited during startup" >&2
    wait "$SERVER_PID" || true
    exit 1
  fi
  i=$((i + 1))
  sleep 1
done

touch "$READY_FILE"
set +e
wait "$SERVER_PID"
STATUS=$?
set -e
trap - TERM INT
rm -f "$READY_FILE"
exit "$STATUS"
