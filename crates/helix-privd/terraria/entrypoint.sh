#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/logs /data/steamcmd /data/worlds /data/mods

FLAVOR="${HELIX_TERRARIA_SOFTWARE:-vanilla}"
PORT="${HELIX_GAME_PORT:-7777}"
PLAYERS="${HELIX_MAX_PLAYERS:-8}"
NAME="${HELIX_SERVER_NAME:-Helix Terraria}"

write_config() {
  cat > /data/serverconfig.txt <<EOF
world=/data/worlds/world.wld
worldname=Helix
maxplayers=${PLAYERS}
port=${PORT}
password=
motd=Hosted by Helix
autocreate=2
difficulty=0
secure=1
lang=en-US
worldpath=/data/worlds
EOF
}

write_config

if [ "$FLAVOR" = "tmodloader" ]; then
  if [ ! -x /data/steamcmd/steamcmd.sh ]; then
    cp -a /opt/steamcmd/. /data/steamcmd/
  fi
  echo "Helix: updating tModLoader dedicated server through SteamCMD" >&2
  /data/steamcmd/steamcmd.sh \
    +@sSteamCmdForcePlatformType linux \
    +force_install_dir /data/server \
    +login anonymous \
    +app_update 1281930 validate \
    +quit
  mkdir -p /data/tModLoader/Mods
  if [ -d /data/mods ]; then
    find /data/mods -maxdepth 1 -type f -name '*.tmod' -exec cp -a {} /data/tModLoader/Mods/ \; 2>/dev/null || true
  fi
  export HOME=/data
  START=""
  for candidate in \
    /data/server/start-tModLoaderServer.sh \
    /data/server/LaunchUtils/ScriptCaller.sh
  do
    if [ -f "$candidate" ]; then
      START="$candidate"
      break
    fi
  done
  if [ -z "$START" ]; then
    echo "Helix: tModLoader dedicated start script was not installed" >&2
    exit 1
  fi
  chmod +x "$START" || true
  cd /data/server
  sh "$START" -config /data/serverconfig.txt >/data/logs/terraria.log 2>&1 &
  SERVER_PID=$!
else
  VERSION="${HELIX_TERRARIA_VERSION:-1449}"
  ZIP="/data/server/terraria-server-${VERSION}.zip"
  if [ ! -f "$ZIP" ]; then
    echo "Helix: downloading Terraria dedicated server ${VERSION}" >&2
    curl -fsSL -o "$ZIP" "https://terraria.org/api/download/pc-dedicated-server/terraria-server-${VERSION}.zip"
  fi
  if [ ! -x /data/server/Linux/TerrariaServer.bin.x86_64 ] && [ ! -x /data/server/${VERSION}/Linux/TerrariaServer.bin.x86_64 ]; then
    unzip -o "$ZIP" -d /data/server >/data/logs/terraria-extract.log 2>&1
  fi
  BIN=""
  for candidate in \
    /data/server/${VERSION}/Linux/TerrariaServer.bin.x86_64 \
    /data/server/Linux/TerrariaServer.bin.x86_64
  do
    if [ -f "$candidate" ]; then
      BIN="$candidate"
      break
    fi
  done
  if [ -z "$BIN" ]; then
    echo "Helix: Terraria dedicated binary was not found in the publisher zip" >&2
    exit 1
  fi
  chmod +x "$BIN"
  cd "$(dirname "$BIN")"
  "./$(basename "$BIN")" -config /data/serverconfig.txt >/data/logs/terraria.log 2>&1 &
  SERVER_PID=$!
fi

i=0
while [ "$i" -lt 40 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Terraria exited during startup" >&2
    tail -n 40 /data/logs/terraria.log >&2 || true
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
rm -f "$READY_FILE"
exit "$STATUS"
