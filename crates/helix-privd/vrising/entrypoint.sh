#!/bin/sh
set -eu

READY_FILE=/data/.helix-ready
rm -f "$READY_FILE"

export WINEPREFIX=/data/wine
export WINEARCH=win64
export WINEDLLOVERRIDES="winhttp=n,b;mshtml=d"
export WINEDEBUG="-all"
export DISPLAY=:99

mkdir -p /data/server /data/save/Settings /data/logs /data/steamcmd "$WINEPREFIX"

if [ ! -x /data/steamcmd/steamcmd.sh ]; then
  cp -a /opt/steamcmd/. /data/steamcmd/
fi

echo "Helix: updating V Rising dedicated server through SteamCMD" >&2
/data/steamcmd/steamcmd.sh \
  +@sSteamCmdForcePlatformType windows \
  +force_install_dir /data/server \
  +login anonymous \
  +app_update 1829350 validate \
  +quit

if [ ! -f /data/server/VRisingServer.exe ]; then
  echo "Helix: VRisingServer.exe was not installed by SteamCMD" >&2
  exit 1
fi

Xvfb :99 -screen 0 640x480x24 -nolisten tcp >/data/logs/xvfb.log 2>&1 &
XVFB_PID=$!
sleep 1
if ! kill -0 "$XVFB_PID" 2>/dev/null; then
  echo "Helix: Xvfb failed to start" >&2
  cat /data/logs/xvfb.log >&2 || true
  exit 1
fi

wineboot --init >/data/logs/wineboot.log 2>&1 || true

cd /data/server
wine VRisingServer.exe \
  -persistentDataPath /data/save \
  -logFile /data/logs/VRisingServer.log \
  -serverName "${HELIX_SERVER_NAME:-Helix V Rising}" &
WINE_PID=$!

i=0
while [ "$i" -lt 40 ]; do
  if ! kill -0 "$WINE_PID" 2>/dev/null; then
    echo "Helix: V Rising exited during startup" >&2
    wait "$WINE_PID" || true
    kill "$XVFB_PID" 2>/dev/null || true
    exit 1
  fi
  i=$((i + 1))
  sleep 1
done

touch "$READY_FILE"
set +e
wait "$WINE_PID"
STATUS=$?
set -e
rm -f "$READY_FILE"
kill "$XVFB_PID" 2>/dev/null || true
wait "$XVFB_PID" 2>/dev/null || true
exit "$STATUS"
