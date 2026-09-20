#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/data

if [ ! -f /data/server/VintagestoryServer.dll ]; then
  echo "Helix: downloading the Vintage Story dedicated server" >&2
  version=$(curl -fsSL https://api.vintagestory.at/lateststable.txt | tr -d ' \r\n')
  if [ -z "$version" ]; then
    echo "Helix: could not resolve the latest stable Vintage Story version" >&2
    exit 1
  fi
  curl -fsSL -o /tmp/vs_server.tar.gz \
    "https://cdn.vintagestory.at/gamefiles/stable/vs_server_linux-x64_${version}.tar.gz"
  tar -xzf /tmp/vs_server.tar.gz -C /data/server
  rm -f /tmp/vs_server.tar.gz
fi

if [ ! -f /data/server/VintagestoryServer.dll ]; then
  echo "Helix: VintagestoryServer.dll was not installed" >&2
  exit 1
fi

GAME_PORT=${HELIX_GAME_PORT:-42420}
MAX_PLAYERS=${HELIX_MAX_PLAYERS:-16}
ADVERTISE=true
if [ "${HELIX_LIST_ON_BROWSER:-true}" = "false" ]; then
  ADVERTISE=false
fi

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

CONFIG=/data/data/serverconfig.json
if [ ! -f "$CONFIG" ]; then
  cat > "$CONFIG" <<EOF
{
  "ServerName": "$(json_escape "${HELIX_SERVER_NAME:-Helix Vintage Story}")",
  "Port": $GAME_PORT,
  "MaxClients": $MAX_PLAYERS,
  "Password": "$(json_escape "${HELIX_SERVER_PASSWORD:-}")",
  "AdvertiseServer": $ADVERTISE
}
EOF
else
  set_json_string() {
    sed -i "s|\"$1\"[[:space:]]*:[[:space:]]*\"[^\"]*\"|\"$1\": \"$2\"|" "$CONFIG"
  }
  set_json_number() {
    sed -i "s|\"$1\"[[:space:]]*:[[:space:]]*[0-9]*|\"$1\": $2|" "$CONFIG"
  }
  set_json_bool() {
    sed -i "s|\"$1\"[[:space:]]*:[[:space:]]*[a-z]*|\"$1\": $2|" "$CONFIG"
  }
  set_json_string ServerName "$(json_escape "${HELIX_SERVER_NAME:-Helix Vintage Story}")"
  set_json_number Port "$GAME_PORT"
  set_json_number MaxClients "$MAX_PLAYERS"
  set_json_string Password "$(json_escape "${HELIX_SERVER_PASSWORD:-}")"
  set_json_bool AdvertiseServer "$ADVERTISE"
fi

cd /data/server

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

dotnet /data/server/VintagestoryServer.dll --dataPath /data/data &
SERVER_PID=$!

i=0
while [ "$i" -lt 90 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Vintage Story exited during startup" >&2
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
