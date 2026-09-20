#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

CLUSTER_ROOT=/data/DoNotStarveTogether
CLUSTER=$CLUSTER_ROOT/Cluster_1

mkdir -p /data/server /data/steamcmd "$CLUSTER/Master"

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

if [ -f /data/steamcmd/linux64/steamclient.so ]; then
  mkdir -p /data/.steam/sdk64
  cp -f /data/steamcmd/linux64/steamclient.so /data/.steam/sdk64/steamclient.so
fi

echo "Helix: updating Don't Starve Together dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 343050 validate \
  +quit

BINARY=/data/server/bin64/dontstarve_dedicated_server_nullrenderer_x64
if [ ! -x "$BINARY" ]; then
  BINARY=/data/server/bin/dontstarve_dedicated_server_nullrenderer
fi
if [ ! -x "$BINARY" ]; then
  echo "Helix: the Don't Starve Together dedicated binary was not installed" >&2
  exit 1
fi
chmod +x "$BINARY" 2>/dev/null || true

GAME_PORT=${HELIX_GAME_PORT:-10999}
CAVES_PORT=${HELIX_QUERY_PORT:-11000}
CAVES=${HELIX_CAVES:-false}
LISTED=${HELIX_LIST_ON_BROWSER:-true}

ini_escape() {
  printf '%s' "$1" | tr -d '\r\n[]' | sed 's/[=;]//g'
}

if [ ! -f "$CLUSTER/cluster.ini" ]; then
  cat > "$CLUSTER/cluster.ini" <<EOF
[GAMEPLAY]
game_mode = survival
max_players = ${HELIX_MAX_PLAYERS:-8}
pvp = false
pause_when_empty = true

[NETWORK]
cluster_name = $(ini_escape "${HELIX_SERVER_NAME:-Helix DST}")
cluster_description =
cluster_password = $(ini_escape "${HELIX_SERVER_PASSWORD:-}")
cluster_intention = cooperative

[MISC]
console_enabled = true

[SHARD]
shard_enabled = $CAVES
is_master = true
name = Master
EOF
fi

if [ -n "${HELIX_CLUSTER_TOKEN:-}" ] && [ ! -s "$CLUSTER/cluster_token.txt" ]; then
  printf '%s\n' "$HELIX_CLUSTER_TOKEN" > "$CLUSTER/cluster_token.txt"
fi

write_shard() {
  shard=$1
  port=$2
  master=$3
  steam_auth=$4
  steam_master=$5
  mkdir -p "$CLUSTER/$shard"
  cat > "$CLUSTER/$shard/server.ini" <<EOF
[NETWORK]
server_port = $port

[SHARD]
is_master = $master
name = $shard
id = $shard

[STEAM]
authentication_port = $steam_auth
master_server_port = $steam_master
EOF
  if [ ! -f "$CLUSTER/$shard/modoverrides.lua" ]; then
    printf 'return {}\n' > "$CLUSTER/$shard/modoverrides.lua"
  fi
  if [ ! -f "$CLUSTER/$shard/worldgenoverride.lua" ]; then
    if [ "$shard" = "Caves" ]; then
      printf 'return { override_enabled = true, preset = "DST_CAVE", overrides = {} }\n' \
        > "$CLUSTER/$shard/worldgenoverride.lua"
    else
      printf 'return { override_enabled = true, preset = "SURVIVAL_TOGETHER", overrides = {} }\n' \
        > "$CLUSTER/$shard/worldgenoverride.lua"
    fi
  fi
}

write_shard Master "$GAME_PORT" true 8766 27016
if [ "$CAVES" = "true" ]; then
  write_shard Caves "$CAVES_PORT" false 8767 27017
fi

cd /data/server

PIDS=""
forward_stop() {
  for pid in $PIDS; do
    kill -TERM "$pid" 2>/dev/null || true
  done
}
trap forward_stop TERM INT

EXTRA=""
if [ "$LISTED" = "false" ] || [ ! -s "$CLUSTER/cluster_token.txt" ]; then
  EXTRA="-offline"
fi

"$BINARY" \
  -persistent_storage_root /data \
  -conf_dir DoNotStarveTogether \
  -cluster Cluster_1 \
  -shard Master \
  $EXTRA &
PIDS="$!"

if [ "$CAVES" = "true" ]; then
  "$BINARY" \
    -persistent_storage_root /data \
    -conf_dir DoNotStarveTogether \
    -cluster Cluster_1 \
    -shard Caves \
    $EXTRA &
  PIDS="$PIDS $!"
fi

i=0
while [ "$i" -lt 120 ]; do
  alive=0
  for pid in $PIDS; do
    if kill -0 "$pid" 2>/dev/null; then
      alive=$((alive + 1))
    fi
  done
  if [ "$alive" -eq 0 ]; then
    echo "Helix: Don't Starve Together exited during startup" >&2
    exit 1
  fi
  i=$((i + 1))
  sleep 1
done

touch "$READY_FILE"
set +e
for pid in $PIDS; do
  wait "$pid"
done
set -e
trap - TERM INT
rm -f "$READY_FILE"
exit 0
