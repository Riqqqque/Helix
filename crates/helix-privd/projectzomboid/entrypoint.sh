#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

SERVER_NAME=helixserver
CACHEDIR=/data/zomboid
INI_DIR="$CACHEDIR/Server"

mkdir -p /data/server /data/steamcmd "$INI_DIR" "$CACHEDIR/Saves/Multiplayer" "$CACHEDIR/db"

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

# Steam query needs the client library where the game looks for it.
if [ -f /data/steamcmd/linux64/steamclient.so ]; then
  mkdir -p /data/.steam/sdk64
  cp -f /data/steamcmd/linux64/steamclient.so /data/.steam/sdk64/steamclient.so
fi

echo "Helix: updating Project Zomboid dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 380870 validate \
  +quit

if [ ! -f /data/server/start-server.sh ]; then
  echo "Helix: start-server.sh was not installed by SteamCMD" >&2
  exit 1
fi
chmod +x /data/server/start-server.sh 2>/dev/null || true

GAME_PORT=${HELIX_GAME_PORT:-16261}
DATA_PORT=${HELIX_QUERY_PORT:-16262}
MAX_PLAYERS=${HELIX_MAX_PLAYERS:-8}
PUBLIC=${HELIX_LIST_ON_BROWSER:-true}
PASSWORD=${HELIX_SERVER_PASSWORD:-}

# Write or patch the managed server ini; user edits to other keys survive.
INI="$INI_DIR/$SERVER_NAME.ini"
if [ ! -f "$INI" ]; then
  cat > "$INI" <<EOF
PublicName=
DefaultPort=$GAME_PORT
UDPPort=$DATA_PORT
MaxPlayers=$MAX_PLAYERS
Password=$PASSWORD
Public=$PUBLIC
Open=true
PauseEmpty=true
PVP=false
SteamScoreboard=true
DoLuaChecksum=true
EOF
fi

set_ini() {
  key=$1
  value=$2
  if grep -q "^$key=" "$INI"; then
    sed -i "s|^$key=.*|$key=$value|" "$INI"
  else
    printf '%s=%s\n' "$key" "$value" >> "$INI"
  fi
}

set_ini PublicName "${HELIX_SERVER_NAME:-Helix Zomboid}"
set_ini DefaultPort "$GAME_PORT"
set_ini UDPPort "$DATA_PORT"
set_ini MaxPlayers "$MAX_PLAYERS"
set_ini Password "$PASSWORD"
set_ini Public "$PUBLIC"

cd /data/server

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

./start-server.sh \
  -servername "$SERVER_NAME" \
  -cachedir="$CACHEDIR" \
  -adminpassword "${HELIX_ADMIN_PASSWORD:?Helix did not pass an admin password}" \
  "-Xmx${HELIX_MEMORY_MB:-8192}m" &
SERVER_PID=$!

i=0
while [ "$i" -lt 120 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Project Zomboid exited during startup" >&2
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
