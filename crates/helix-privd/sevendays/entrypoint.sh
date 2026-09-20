#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/config /data/logs /data/saves /data/steamcmd

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

echo "Helix: updating 7 Days to Die dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 294420 validate \
  +quit

if [ ! -f /data/server/7DaysToDieServer.x86_64 ]; then
  echo "Helix: 7DaysToDieServer.x86_64 was not installed by SteamCMD" >&2
  exit 1
fi
chmod +x /data/server/7DaysToDieServer.x86_64 2>/dev/null || true

CONFIG=/data/config/helix.xml
if [ ! -f "$CONFIG" ] && [ -f /data/server/serverconfig.xml ]; then
  cp /data/server/serverconfig.xml "$CONFIG"
fi
if [ ! -f "$CONFIG" ]; then
  cat > "$CONFIG" <<'EOF'
<?xml version="1.0"?>
<ServerSettings>
  <property name="ServerName" value="Helix 7 Days to Die"/>
  <property name="ServerPort" value="26900"/>
  <property name="ServerMaxPlayerCount" value="8"/>
  <property name="ServerPassword" value=""/>
  <property name="ServerVisibility" value="2"/>
  <property name="ServerDisabledNetworkProtocols" value=""/>
  <property name="GameWorld" value="Navezgane"/>
  <property name="GameName" value="Helix"/>
  <property name="GameDifficulty" value="2"/>
  <property name="SaveGameFolder" value="/data/saves"/>
  <property name="TelnetEnabled" value="false"/>
  <property name="ControlPanelEnabled" value="false"/>
  <property name="WebDashboardEnabled" value="false"/>
  <property name="EACEnabled" value="false"/>
</ServerSettings>
EOF
fi

xml_escape() {
  printf '%s' "$1" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g; s/"/\&quot;/g'
}

set_prop() {
  key=$1
  value=$(xml_escape "$2")
  if grep -q "name=\"$key\"" "$CONFIG"; then
    sed -i "s|<property name=\"$key\" value=\"[^\"]*\"/>|<property name=\"$key\" value=\"$value\"/>|" "$CONFIG"
  else
    sed -i "s|</ServerSettings>|  <property name=\"$key\" value=\"$value\"/>\n</ServerSettings>|" "$CONFIG"
  fi
}

GAME_PORT=${HELIX_GAME_PORT:-26900}
VISIBILITY=2
if [ "${HELIX_LIST_ON_BROWSER:-true}" = "false" ]; then
  VISIBILITY=1
fi

set_prop ServerName "${HELIX_SERVER_NAME:-Helix 7 Days to Die}"
set_prop ServerPort "$GAME_PORT"
set_prop ServerMaxPlayerCount "${HELIX_MAX_PLAYERS:-8}"
set_prop ServerPassword "${HELIX_SERVER_PASSWORD:-}"
set_prop ServerVisibility "$VISIBILITY"
set_prop SaveGameFolder /data/saves
set_prop TelnetEnabled false
set_prop ControlPanelEnabled false
set_prop WebDashboardEnabled false
if [ -n "${HELIX_WORLD_NAME:-}" ]; then
  set_prop GameName "$HELIX_WORLD_NAME"
fi
if [ -n "${HELIX_WORLD_SEED:-}" ]; then
  set_prop GameWorld RWG
  set_prop WorldGenSeed "$HELIX_WORLD_SEED"
fi

cd /data/server

SERVER_PID=""
forward_stop() {
  if [ -n "$SERVER_PID" ]; then
    kill -TERM "$SERVER_PID" 2>/dev/null || true
  fi
}
trap forward_stop TERM INT

./7DaysToDieServer.x86_64 \
  -quit -batchmode -nographics \
  -configfile="$CONFIG" \
  -dedicated \
  -logfile /data/logs/7d2d.log &
SERVER_PID=$!

i=0
while [ "$i" -lt 120 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: 7 Days to Die exited during startup" >&2
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
