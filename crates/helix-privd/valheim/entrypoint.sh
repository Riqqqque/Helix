#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

mkdir -p /data/server /data/logs /data/steamcmd /data/plugins /data/worlds

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

echo "Helix: updating Valheim dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +@sSteamCmdForcePlatformType linux \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 896660 validate \
  +quit

if [ ! -x /data/server/valheim_server.x86_64 ]; then
  echo "Helix: valheim_server.x86_64 was not installed by SteamCMD" >&2
  exit 1
fi

if [ -f /data/bepinex-pack.zip ]; then
  echo "Helix: applying BepInEx pack" >&2
  unzip -o /data/bepinex-pack.zip -d /data/server >/data/logs/bepinex-extract.log 2>&1 || true
  if [ -d /data/server/BepInExPack_Valheim ]; then
    cp -a /data/server/BepInExPack_Valheim/. /data/server/
  fi
fi

mkdir -p /data/server/BepInEx/plugins
if [ -d /data/plugins ]; then
  find /data/plugins -maxdepth 1 -type f \( -name '*.dll' -o -name '*.zip' \) -exec cp -a {} /data/server/BepInEx/plugins/ \; 2>/dev/null || true
fi

export templdpath="${LD_LIBRARY_PATH:-}"
export LD_LIBRARY_PATH="/data/server/linux64:${LD_LIBRARY_PATH:-}"
export SteamAppId=892970

cd /data/server
./valheim_server.x86_64 \
  -nographics \
  -batchmode \
  -name "${HELIX_SERVER_NAME:-Helix Valheim}" \
  -port "${HELIX_GAME_PORT:-2456}" \
  -world "${HELIX_WORLD_NAME:-Dedicated}" \
  -password "${HELIX_PASSWORD:-}" \
  -public 0 \
  -savedir /data \
  >/data/logs/valheim.log 2>&1 &
SERVER_PID=$!

i=0
while [ "$i" -lt 40 ]; do
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    echo "Helix: Valheim exited during startup" >&2
    tail -n 40 /data/logs/valheim.log >&2 || true
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
