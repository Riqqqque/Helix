#!/bin/bash
# Helix runtime for the official Hytale dedicated server.
#
# Server files come from the official Hytale Downloader, which signs in with a
# Hytale account (OAuth device code). The server itself also signs in once with
# `/auth login device`; `/auth persistence Encrypted` keeps that across restarts.
# Sign-in links are written to /data/.helix/hytale-auth.json for Helix to show.
set -euo pipefail

STATE=/data/.helix
READY_FILE=/data/.helix-ready
AUTH_FILE=$STATE/hytale-auth.json
CONSOLE=$STATE/console.fifo
OUTPUT=$STATE/output.fifo
DOWNLOADER_DIR=$STATE/downloader
VERSION_FILE=$STATE/server-version
DOWNLOADER_URL=https://downloader.hytale.com/hytale-downloader.zip

GAME_PORT=${HELIX_GAME_PORT:-5520}
MEMORY_MB=${HELIX_MEMORY_MB:-6144}
MAX_PLAYERS=${HELIX_MAX_PLAYERS:-16}
SERVER_NAME=${HELIX_SERVER_NAME:-Helix Hytale}
PATCHLINE=${HELIX_PATCHLINE:-release}
AUTO_UPDATE=${HELIX_AUTO_UPDATE:-true}

rm -f "$READY_FILE"
mkdir -p "$STATE" "$DOWNLOADER_DIR" /data/mods /data/universe /data/logs
chmod 700 "$STATE"

log() { printf 'Helix: %s\n' "$*" >&2; }

# ---------------------------------------------------------------------------
# Sign-in prompts. Only characters that are safe inside a JSON string are kept.
# ---------------------------------------------------------------------------
write_auth() {
  local tmp="$AUTH_FILE.tmp"
  printf '{"stage":"%s","state":"%s","url":"%s","code":"%s","updated_at":%s}\n' \
    "$1" "$2" "$3" "$4" "$(date +%s)" >"$tmp"
  mv -f "$tmp" "$AUTH_FILE"
}

AUTH_URL=""
AUTH_CODE=""
capture_auth() {
  local stage=$1 line=$2 url code
  url=$(printf '%s' "$line" | grep -oE 'https://([A-Za-z0-9-]+\.)*hytale\.com/[A-Za-z0-9./?=&_%:~+-]*' | head -n 1 || true)
  code=$(printf '%s' "$line" | grep -oE 'user_code=[A-Za-z0-9-]{4,32}' | head -n 1 | cut -d= -f2 || true)
  if [ -z "$code" ]; then
    code=$(printf '%s' "$line" | grep -oiE '(user )?code[^A-Za-z0-9]{1,4}[A-Za-z0-9-]{4,32}' | grep -oE '[A-Za-z0-9-]{4,32}$' | head -n 1 || true)
  fi
  if [ -n "$url" ]; then
    # Prefer the link that already carries the code over a bare verify page.
    case "$url" in
      *user_code=*) AUTH_URL=$url ;;
      *) case "$AUTH_URL" in *user_code=*) ;; *) AUTH_URL=$url ;; esac ;;
    esac
  fi
  if [ -n "$code" ]; then
    AUTH_CODE=$code
  fi
  if [ -n "$url$code" ] && [ -n "$AUTH_URL" ]; then
    write_auth "$stage" needs_sign_in "$AUTH_URL" "$AUTH_CODE"
  fi
}

clear_auth() {
  AUTH_URL=""
  AUTH_CODE=""
  write_auth "$1" signed_in "" ""
}

# ---------------------------------------------------------------------------
# Server files
# ---------------------------------------------------------------------------
downloader_path() {
  find "$DOWNLOADER_DIR" -maxdepth 2 -type f -name 'hytale-downloader-linux-amd64' | head -n 1
}

ensure_downloader() {
  local path
  path=$(downloader_path)
  if [ -z "$path" ]; then
    log "fetching the official Hytale Downloader"
    curl -fsSL --proto '=https' --tlsv1.2 -o "$STATE/hytale-downloader.zip" "$DOWNLOADER_URL"
    unzip -o -q "$STATE/hytale-downloader.zip" -d "$DOWNLOADER_DIR"
    rm -f "$STATE/hytale-downloader.zip"
    path=$(downloader_path)
  fi
  if [ -z "$path" ]; then
    log "the Hytale Downloader archive did not contain a Linux x86-64 binary"
    return 1
  fi
  chmod 0755 "$path"
  printf '%s' "$path"
}

run_downloader() {
  local downloader=$1
  shift
  (cd "$DOWNLOADER_DIR" && HOME="$DOWNLOADER_DIR" "$downloader" -patchline "$PATCHLINE" "$@")
}

install_or_update() {
  local downloader latest="" current="" attempt
  downloader=$(ensure_downloader)
  current=$(cat "$VERSION_FILE" 2>/dev/null || true)
  if [ -f "$DOWNLOADER_DIR/.hytale-downloader-credentials.json" ]; then
    latest=$(run_downloader "$downloader" -print-version 2>/dev/null | tr -d '\r' | tail -n 1 | tr -d ' ' || true)
  fi
  if [ -f /data/Server/HytaleServer.jar ] && [ -f /data/Assets.zip ]; then
    if [ "$AUTO_UPDATE" != "true" ] || [ -z "$latest" ] || [ "$latest" = "$current" ]; then
      return 0
    fi
    log "updating the Hytale server from ${current:-an unknown version} to $latest"
  else
    log "downloading the Hytale server${latest:+ $latest}"
  fi

  rm -rf "$STATE/stage" "$STATE/game.zip"
  for attempt in 1 2 3; do
    if run_downloader "$downloader" -download-path "$STATE/game.zip" 2>&1 \
      | while IFS= read -r line; do
          printf '%s\n' "$line"
          capture_auth download "$line"
          case "$line" in
            # The downloader only announces its own updates; drop the cached
            # binary (credentials stay) so the next start fetches the new one.
            *"new version of hytale-downloader is available"*)
              rm -f "$downloader"
              ;;
          esac
        done && [ -s "$STATE/game.zip" ]; then
      break
    fi
    rm -f "$STATE/game.zip"
    if [ "$attempt" = 3 ]; then
      if [ -f /data/Server/HytaleServer.jar ] && [ -f /data/Assets.zip ]; then
        log "the update download failed; starting the installed version"
        return 0
      fi
      log "the Hytale server download failed"
      return 1
    fi
    log "the download did not finish (attempt $attempt of 3); retrying"
    sleep 5
  done
  write_auth download signed_in "" ""

  mkdir -p "$STATE/stage"
  unzip -q "$STATE/game.zip" -d "$STATE/stage"
  rm -f "$STATE/game.zip"
  if [ ! -f "$STATE/stage/Server/HytaleServer.jar" ] || [ ! -f "$STATE/stage/Assets.zip" ]; then
    rm -rf "$STATE/stage"
    log "the downloaded archive is missing Server/HytaleServer.jar or Assets.zip"
    [ -f /data/Server/HytaleServer.jar ] && return 0
    return 1
  fi
  rm -rf /data/Server.previous
  [ -d /data/Server ] && mv /data/Server /data/Server.previous
  mv "$STATE/stage/Server" /data/Server
  mv -f "$STATE/stage/Assets.zip" /data/Assets.zip
  rm -rf "$STATE/stage" /data/Server.previous
  if [ -z "$latest" ]; then
    latest=$(run_downloader "$downloader" -print-version 2>/dev/null | tr -d '\r' | tail -n 1 | tr -d ' ' || true)
  fi
  [ -n "$latest" ] && printf '%s\n' "$latest" >"$VERSION_FILE"
  log "Hytale server ${latest:-files} installed"
}

# ---------------------------------------------------------------------------
# config.json: only the keys Helix manages are touched.
# ---------------------------------------------------------------------------
apply_config() {
  local config=/data/config.json tmp=/data/config.json.helix
  if [ ! -f "$config" ]; then
    printf '{"Version":3,"ServerName":"","MOTD":"","Password":"","MaxPlayers":16,"MaxViewRadius":12,"Defaults":{"World":"default","GameMode":"Adventure"}}\n' >"$config"
  fi
  if jq --arg name "$SERVER_NAME" --arg password "${HELIX_SERVER_PASSWORD:-}" \
        --argjson players "$MAX_PLAYERS" \
        '.ServerName = $name | .MaxPlayers = $players | .Password = $password' \
        "$config" >"$tmp"; then
    mv -f "$tmp" "$config"
  else
    rm -f "$tmp"
    log "config.json is not valid JSON; Helix left it unchanged"
  fi
}

install_or_update
apply_config

# ---------------------------------------------------------------------------
# Run the server with a console pipe Helix can write commands into.
# ---------------------------------------------------------------------------
cd /data
rm -f "$CONSOLE" "$OUTPUT"
mkfifo -m 600 "$CONSOLE" "$OUTPUT"
# Hold the console open read-write so the server never sees end-of-input and
# Helix's non-blocking writes always find a reader.
exec 3<>"$CONSOLE"

send() { printf '%s\n' "$1" >&3; }

HEAP_MB=$MEMORY_MB
MIN_HEAP_MB=$((MEMORY_MB / 2))
JAVA_ARGS=("-Xms${MIN_HEAP_MB}M" "-Xmx${HEAP_MB}M")
if [ -f /data/Server/HytaleServer.aot ]; then
  JAVA_ARGS+=("-XX:AOTCache=/data/Server/HytaleServer.aot")
fi

java "${JAVA_ARGS[@]}" -jar /data/Server/HytaleServer.jar \
  --assets /data/Assets.zip \
  --bind "0.0.0.0:${GAME_PORT}" \
  --auth-mode authenticated \
  <&3 >"$OUTPUT" 2>&1 &
SERVER_PID=$!

watch_server() {
  local line
  while IFS= read -r line; do
    printf '%s\n' "$line"
    capture_auth server "$line"
    case "$line" in
      *"Hytale Server Booted!"*)
        touch "$READY_FILE"
        if [ ! -f /data/auth.enc ]; then
          send "/auth login device"
        fi
        ;;
      *"Multiple profiles available"*)
        send "/auth select 1"
        ;;
      *"Authentication successful"*|*"already authenticated"*)
        send "/auth persistence Encrypted"
        clear_auth server
        ;;
    esac
  done
}
watch_server <"$OUTPUT" &
WATCH_PID=$!

STOPPING=0
# shellcheck disable=SC2329 # invoked by the TERM/INT trap below
forward_stop() {
  STOPPING=1
  send "/stop" || true
  local _
  for _ in $(seq 1 90); do
    kill -0 "$SERVER_PID" 2>/dev/null || return 0
    sleep 1
  done
  kill -TERM "$SERVER_PID" 2>/dev/null || true
}
trap forward_stop TERM INT

# Treat a server that stays up for three minutes as ready even if the boot
# message changes in a future Hytale release.
elapsed=0
while kill -0 "$SERVER_PID" 2>/dev/null && [ ! -f "$READY_FILE" ] && [ "$STOPPING" = 0 ]; do
  sleep 1
  elapsed=$((elapsed + 1))
  if [ "$elapsed" -ge 180 ]; then
    touch "$READY_FILE"
  fi
done

STATUS=0
while kill -0 "$SERVER_PID" 2>/dev/null; do
  wait "$SERVER_PID" && STATUS=0 || STATUS=$?
done
trap - TERM INT
rm -f "$READY_FILE"
exec 3>&-
kill "$WATCH_PID" 2>/dev/null || true
exit "$STATUS"
