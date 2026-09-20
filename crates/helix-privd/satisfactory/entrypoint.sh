#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/steamcmd

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

echo "Helix: updating Satisfactory dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 1690800 validate \
  +quit

if [ ! -f /data/server/FactoryServer.sh ]; then
  echo "Helix: FactoryServer.sh was not installed by SteamCMD" >&2
  exit 1
fi
chmod +x /data/server/FactoryServer.sh 2>/dev/null || true

cd /data/server

set -- \
  "-Port=${HELIX_GAME_PORT:-7777}" \
  "-BeaconPort=${HELIX_QUERY_PORT:-15000}" \
  "-ini:Game:[/Script/Engine.GameSession]:MaxPlayers=${HELIX_MAX_PLAYERS:-4}" \
  -multihome=0.0.0.0 \
  -unattended \
  -log

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

./FactoryServer.sh "$@" &
SERVER_PID=$!

i=0
while [ "$i" -lt 90 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Satisfactory exited during startup" >&2
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
