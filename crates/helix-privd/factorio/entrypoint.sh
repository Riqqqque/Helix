#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/config /data/saves /data/mods

FACTORIO_BIN=/data/server/factorio/bin/x64/factorio

if [ ! -x "$FACTORIO_BIN" ]; then
  echo "Helix: downloading the Factorio headless server" >&2
  version=$(curl -fsSL https://factorio.com/api/latest-releases \
    | sed -n 's/.*"stable"[^{]*{[^}]*"headless"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n1)
  if [ -n "$version" ]; then
    url="https://www.factorio.com/get-download/$version/headless/linux64"
  else
    url="https://www.factorio.com/get-download/stable/headless/linux64"
  fi
  curl -fsSL -o /tmp/factorio.tar.xz "$url"
  tar -xJf /tmp/factorio.tar.xz -C /data/server
  rm -f /tmp/factorio.tar.xz
fi

if [ ! -x "$FACTORIO_BIN" ]; then
  echo "Helix: the Factorio headless binary was not installed" >&2
  exit 1
fi

# Keep managed folders outside the install tree so updates never touch data.
for folder in saves mods config; do
  if [ ! -L "/data/server/factorio/$folder" ]; then
    rm -rf "/data/server/factorio/$folder"
    ln -s "/data/$folder" "/data/server/factorio/$folder"
  fi
done

json_escape() {
  printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

PUBLIC=true
if [ "${HELIX_LIST_ON_BROWSER:-true}" = "false" ]; then
  PUBLIC=false
fi

cat > /data/config/server-settings.json <<EOF
{
  "name": "$(json_escape "${HELIX_SERVER_NAME:-Helix Factorio}")",
  "description": "Helix-managed Factorio server",
  "tags": ["helix"],
  "max_players": ${HELIX_MAX_PLAYERS:-16},
  "visibility": { "public": $PUBLIC, "lan": true },
  "username": "",
  "password": "",
  "token": "",
  "game_password": "$(json_escape "${HELIX_SERVER_PASSWORD:-}")",
  "require_user_verification": true,
  "max_upload_in_kilobytes_per_second": 0,
  "max_upload_slots": 5,
  "minimum_latency_in_ticks": 0,
  "ignore_player_limit_for_returning_players": false,
  "allow_commands": "admins-only",
  "autosave_interval": 10,
  "autosave_slots": 5,
  "afk_autokick_interval": 0,
  "auto_pause": true,
  "only_admins_can_pause_the_game": false,
  "autosave_only_on_server": true,
  "non_blocking_saving": true
}
EOF

if ! find /data/saves -name '*.zip' -print -quit | grep -q .; then
  echo "Helix: creating the first Factorio save" >&2
  "$FACTORIO_BIN" --create /data/saves/helix.zip
fi

cd /data/server/factorio

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

"$FACTORIO_BIN" \
  --start-server-load-latest \
  --server-settings /data/config/server-settings.json \
  --port "${HELIX_GAME_PORT:-34197}" &
SERVER_PID=$!

i=0
while [ "$i" -lt 60 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Factorio exited during startup" >&2
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
