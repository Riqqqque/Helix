#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/userdata /data/wine /data/steamcmd

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

export WINEPREFIX=/data/wine
export WINEARCH=win64
export WINEDEBUG=-all
export HOME=/data

echo "Helix: updating Sons of the Forest dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +@sSteamCmdForcePlatformType windows \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 2465200 validate \
  +quit

if [ ! -f /data/server/SonsOfTheForestDS.exe ]; then
  echo "Helix: SonsOfTheForestDS.exe was not installed by SteamCMD" >&2
  exit 1
fi

GAME_PORT=${HELIX_GAME_PORT:-8766}
QUERY_PORT=${HELIX_QUERY_PORT:-27016}
BLOB_PORT=${HELIX_AUX_PORT:-9700}

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

cat > /data/userdata/dedicatedserver.cfg <<EOF
{
  "IpAddress": "0.0.0.0",
  "GamePort": $GAME_PORT,
  "QueryPort": $QUERY_PORT,
  "BlobSyncPort": $BLOB_PORT,
  "ServerName": "$(json_escape "${HELIX_SERVER_NAME:-Helix Sons of the Forest}")",
  "MaxPlayers": ${HELIX_MAX_PLAYERS:-8},
  "Password": "$(json_escape "${HELIX_SERVER_PASSWORD:-}")",
  "LanOnly": false,
  "SaveSlot": 1,
  "GameMode": "Normal",
  "SkipNetworkAccessibilityTest": true,
  "EnableLog": true
}
EOF

cd /data/server

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

xvfb-run -a wine SonsOfTheForestDS.exe -userdatapath 'Z:\data\userdata' &
SERVER_PID=$!

# Wine + prefix initialization makes first boot slow.
i=0
while [ "$i" -lt 240 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Sons of the Forest exited during startup" >&2
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
