#!/bin/sh
set -eu
trap 'status=$?; if [ "$status" -ne 0 ]; then echo "Fixture failed: $status" >&2; cat /data/launcher-test.log >&2; ls -la /data/server >&2; fi' EXIT
/usr/local/bin/helix-vrising >/data/launcher-test.log 2>&1 &
pid=$!
i=0
while [ ! -f /data/.helix-ready ] || [ ! -f /data/server/test-ready ]; do
  if ! kill -0 "$pid" 2>/dev/null || [ "$i" -ge 90 ]; then
    cat /data/launcher-test.log /data/logs/wineboot.log >&2
    exit 1
  fi
  i=$((i + 1))
  sleep 1
done
kill -TERM "$pid"
timeout 20 tail --pid="$pid" -f /dev/null
wait "$pid"
test "$(cat /data/server/test-saved)" = saved
test ! -e /data/.helix-ready
echo 'V Rising wrapper and Wine Ctrl+C save fixture passed'
