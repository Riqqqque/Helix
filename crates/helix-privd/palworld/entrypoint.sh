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

echo "Helix: updating Palworld dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 2394010 validate \
  +quit

if [ ! -f /data/server/PalServer.sh ]; then
  echo "Helix: PalServer.sh was not installed by SteamCMD" >&2
  exit 1
fi
chmod +x /data/server/PalServer.sh 2>/dev/null || true

cd /data/server

set -- \
  "-port=${HELIX_GAME_PORT:-8211}" \
  "-queryport=${HELIX_QUERY_PORT:-27015}" \
  "-players=${HELIX_MAX_PLAYERS:-32}" \
  "-servername=${HELIX_SERVER_NAME:-Helix Palworld}" \
  "-adminpassword=${HELIX_ADMIN_PASSWORD:?Helix did not pass an admin password}" \
  -useperfthreads \
  -NoAsyncLoadingThread \
  -UseMultithreadForDS \
  -EpicApp=PalServer

if [ "${HELIX_LIST_ON_BROWSER:-true}" = "true" ]; then
  set -- "$@" -publiclobby
fi
if [ -n "${HELIX_SERVER_PASSWORD:-}" ]; then
  set -- "$@" "-serverpassword=${HELIX_SERVER_PASSWORD}"
fi

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

./PalServer.sh "$@" &
SERVER_PID=$!

i=0
while [ "$i" -lt 60 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Palworld exited during startup" >&2
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
