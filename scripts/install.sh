#!/usr/bin/env bash
# Helix installer for headless Linux servers.
#
#   curl -fsSL https://raw.githubusercontent.com/Riqqqque/Helix/main/scripts/install.sh | sudo bash
#
# Installs the complete private-LAN deployment: the dashboard and gateway
# containers, the typed host broker (helix-privd), its systemd units, and the
# configuration they need. It downloads a SHA-256-verified release, detects this
# server's LAN address, and prints the dashboard URL and a one-time owner token.
#
# Run with --help for options. Re-running is safe: it repairs or upgrades the
# install and keeps your data and settings.
set -Eeuo pipefail

HELIX_REPO="${HELIX_REPO:-Riqqqque/Helix}"
INSTALL_ROOT="/opt/helix"
ENV_FILE="$INSTALL_ROOT/helix.env"
SRV_ROOT="/srv/helix"
PRIVD_CONFIG="/etc/helix/privd.json"
PRIVD_BIN="/usr/local/libexec/helix-privd"
TERMINALD_BIN="/usr/local/libexec/helix-terminald"
UNIT_DIR="/etc/systemd/system"
PROJECT="server-dashboard"
DASHBOARD_UID=10001
DEFAULT_PORT=3100

MODE="install"
ASSUME_YES=0
SOURCE_DIR=""
VERSION=""
LAN_IP=""
LAN_CIDR=""
PORT=""
STORAGE_ROOTS=()
TERMINAL_USER=""
INSTALL_DOCKER=0
OPEN_FIREWALL=""
RECONFIGURE=0
NEW_TOKEN=0
TOKEN_FILE=""
PURGE=0
ORIGINAL_ARGS=("$@")

if [ -t 1 ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RED=$'\033[31m'; RESET=$'\033[0m'
else
  BOLD=""; DIM=""; GREEN=""; YELLOW=""; RED=""; RESET=""
fi

say()  { printf '%s\n' "$*"; }
step() { printf '\n%s==>%s %s%s%s\n' "$GREEN" "$RESET" "$BOLD" "$*" "$RESET"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '%s!%s   %s\n' "$YELLOW" "$RESET" "$*" >&2; }
die()  { printf '%sError:%s %s\n' "$RED" "$RESET" "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Helix installer for headless Linux servers

Usage: install.sh [options]

Install (default):
  --yes                    Accept defaults and answer yes; needed when no terminal is attached
  --version vX.Y.Z         Install a specific release instead of the latest
  --source DIR             Install from an existing Helix source tree (for development)
  --lan-ip ADDRESS         This server's private IPv4 address (default: detected)
  --cidr NETWORK/BITS      Client network allowed to open Helix (default: detected subnet)
  --port PORT              Dashboard port on the LAN address (default: 3100)
  --storage DIR            Extra folder Helix may browse and manage; repeatable
  --terminal-user USER     Enable the in-browser terminal for an existing non-root login
  --install-docker         Install Docker with Docker's official script if it is missing
  --open-firewall          Add a UFW rule for the dashboard port when UFW is active
  --reconfigure            Rewrite the generated configuration instead of keeping it
  --new-token              Issue a fresh owner setup token (only before an owner exists)
  --token-file FILE        Write the owner setup token to FILE instead of printing it

Other modes:
  --check                  Diagnose an existing install and exit
  --uninstall              Remove Helix services and containers; keeps game servers and data
  --purge                  With --uninstall, also delete Helix data, configuration, and backups
  -h, --help               Show this help

Game servers, worlds, and backups are never deleted unless you use --purge and confirm.
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --yes|-y) ASSUME_YES=1 ;;
    --version) VERSION="${2:?--version needs a value}"; shift ;;
    --source) SOURCE_DIR="${2:?--source needs a directory}"; shift ;;
    --lan-ip) LAN_IP="${2:?--lan-ip needs an address}"; shift ;;
    --cidr) LAN_CIDR="${2:?--cidr needs a network}"; shift ;;
    --port) PORT="${2:?--port needs a number}"; shift ;;
    --storage) STORAGE_ROOTS+=("${2:?--storage needs a directory}"); shift ;;
    --terminal-user) TERMINAL_USER="${2:?--terminal-user needs a login}"; shift ;;
    --install-docker) INSTALL_DOCKER=1 ;;
    --open-firewall) OPEN_FIREWALL=1 ;;
    --reconfigure) RECONFIGURE=1 ;;
    --new-token) NEW_TOKEN=1 ;;
    --token-file) TOKEN_FILE="${2:?--token-file needs a path}"; shift ;;
    --check) MODE="check" ;;
    --uninstall) MODE="uninstall" ;;
    --purge) PURGE=1 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
  shift
done

# Prompts read from the terminal so `curl … | sudo bash` can still ask questions.
TTY=""
if [ -r /dev/tty ] && [ -w /dev/tty ] && { : < /dev/tty; } 2>/dev/null; then TTY=/dev/tty; fi

confirm() {
  # confirm "question" default(y|n)
  local question="$1" default="${2:-y}" answer hint
  if [ "$ASSUME_YES" -eq 1 ]; then [ "$default" = "y" ]; return; fi
  [ -n "$TTY" ] || die "no terminal for the question \"$question\"; re-run with --yes and the options you want"
  if [ "$default" = "y" ]; then hint="[Y/n]"; else hint="[y/N]"; fi
  printf '%s %s ' "$question" "$hint" > "$TTY"
  read -r answer < "$TTY" || answer=""
  answer="${answer:-$default}"
  case "$answer" in [Yy]*) return 0 ;; *) return 1 ;; esac
}

prompt_value() {
  # prompt_value "question" default -> echoes the answer
  local question="$1" default="$2" answer
  if [ "$ASSUME_YES" -eq 1 ] || [ -z "$TTY" ]; then printf '%s' "$default"; return; fi
  printf '%s [%s] ' "$question" "$default" > "$TTY"
  read -r answer < "$TTY" || answer=""
  printf '%s' "${answer:-$default}"
}

need_root() { [ "$(id -u)" -eq 0 ] || die "run this as root, for example: curl -fsSL …/install.sh | sudo bash"; }
have() { command -v "$1" >/dev/null 2>&1; }

compose() {
  docker compose --env-file "$ENV_FILE" --project-name "$PROJECT" -f "$SOURCE_DIR/compose.yaml" "$@"
}

env_value() {
  # env_value KEY -> value from the installed env file
  [ -r "$ENV_FILE" ] || return 1
  sed -n "s/^$1=//p" "$ENV_FILE" | tail -n 1
}

# ---------------------------------------------------------------- bootstrap --

source_version() {
  sed -n 's/^version = "\(.*\)"/\1/p' "$1/Cargo.toml" | head -n 1
}

download() {
  # download URL FILE
  if have curl; then curl -fsSL --retry 3 --proto '=https' -o "$2" "$1"
  elif have wget; then wget -q -O "$2" "$1"
  else die "curl or wget is required to download Helix"; fi
}

bootstrap_release() {
  # Fetch a verified release and hand over to the installer inside it, so the
  # installer always matches the compose file and units it configures.
  need_root
  have tar || die "tar is required"
  have sha256sum || die "sha256sum is required"
  local tag="$VERSION" version work
  if [ -z "$tag" ]; then
    step "Finding the latest Helix release"
    local metadata
    metadata="$(mktemp)"
    download "https://api.github.com/repos/$HELIX_REPO/releases/latest" "$metadata"
    tag="$(sed -n 's/.*"tag_name": *"\(v[0-9][0-9.]*\)".*/\1/p' "$metadata" | head -n 1)"
    rm -f "$metadata"
    [ -n "$tag" ] || die "could not read the latest release from GitHub"
  fi
  case "$tag" in v*) ;; *) tag="v$tag" ;; esac
  version="${tag#v}"
  info "Release $tag"
  local target="$INSTALL_ROOT/releases/$version"
  if [ ! -f "$target/compose.yaml" ] || [ ! -f "$target/scripts/install.sh" ]; then
    step "Downloading and verifying Helix $version"
    work="$(mktemp -d)"
    download "https://github.com/$HELIX_REPO/releases/download/$tag/helix-source-$version.tar.gz" "$work/helix-source-$version.tar.gz"
    download "https://github.com/$HELIX_REPO/releases/download/$tag/SHA256SUMS" "$work/SHA256SUMS"
    (cd "$work" && grep " \*\?helix-source-$version.tar.gz\$" SHA256SUMS | sha256sum --check --strict --quiet) \
      || die "the downloaded archive does not match the published SHA-256 checksum"
    info "Checksum verified"
    if tar -tzf "$work/helix-source-$version.tar.gz" | grep -qvE "^helix-$version/"; then
      die "the release archive contains unexpected paths"
    fi
    install -d -m 0755 "$INSTALL_ROOT/releases"
    rm -rf "$target.partial"
    install -d -m 0755 "$target.partial"
    tar -xzf "$work/helix-source-$version.tar.gz" -C "$target.partial" --strip-components=1 --no-same-owner
    rm -rf "$target"
    mv "$target.partial" "$target"
    rm -rf "$work"
  fi
  [ -f "$target/scripts/install.sh" ] || die "Helix $version predates this installer; install $tag or newer with --version, or use the manual guide"
  local forwarded=() skip=0 arg
  for arg in "${ORIGINAL_ARGS[@]}"; do
    if [ "$skip" -eq 1 ]; then skip=0; continue; fi
    case "$arg" in --version) skip=1; continue ;; esac
    forwarded+=("$arg")
  done
  exec bash "$target/scripts/install.sh" --source "$target" "${forwarded[@]}"
}

resolve_source() {
  if [ -n "$SOURCE_DIR" ]; then
    SOURCE_DIR="$(cd "$SOURCE_DIR" && pwd)"
    if [ ! -f "$SOURCE_DIR/compose.yaml" ] || [ ! -f "$SOURCE_DIR/Dockerfile" ]; then
      die "$SOURCE_DIR is not a Helix source tree"
    fi
    return
  fi
  # Running from a checkout or an extracted release: use the tree this script lives in.
  local here
  # Piped from curl there is no script file, so always download a release then.
  here=""
  if [ -f "${BASH_SOURCE[0]:-}" ] && cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null; then here="$(pwd)"; cd - >/dev/null; fi
  if [ -n "$here" ] && [ -f "$here/../compose.yaml" ] && [ -f "$here/../Dockerfile" ] && [ -z "$VERSION" ]; then
    SOURCE_DIR="$(cd "$here/.." && pwd)"
    return
  fi
  bootstrap_release
}

# ---------------------------------------------------------------- checks --

preflight() {
  step "Checking this server"
  need_root
  [ "$(uname -s)" = "Linux" ] || die "Helix needs Linux"
  [ -d /run/systemd/system ] || die "Helix needs systemd as the init system"
  case "$(uname -m)" in x86_64|amd64|aarch64|arm64) ;; *) die "Helix supports 64-bit x86 and ARM; this is $(uname -m)" ;; esac
  local tool
  for tool in ip getent groupadd install sed awk tar sha256sum timeout systemctl; do
    have "$tool" || die "the '$tool' command is missing; install it and re-run"
  done
  have curl || die "curl is missing; install the 'curl' package and re-run (Helix's updater needs it too)"
  # shellcheck disable=SC1091
  info "$(. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-Linux}") on $(uname -m)"
}

ensure_docker() {
  step "Checking Docker"
  if have docker && docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    info "$(docker --version | sed 's/,.*//'), $(docker compose version --short 2>/dev/null | sed 's/^/Compose /')"
    return
  fi
  if have docker && ! docker info >/dev/null 2>&1; then
    systemctl enable --now docker >/dev/null 2>&1 || true
    if docker info >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
      info "Started the Docker service"
      return
    fi
  fi
  if have docker && ! docker compose version >/dev/null 2>&1; then
    warn "Docker is installed but the Compose v2 plugin ('docker compose') is not."
  else
    warn "Docker is not installed."
  fi
  if [ "$INSTALL_DOCKER" -eq 1 ] || { [ "$ASSUME_YES" -eq 0 ] && confirm "Install Docker now with Docker's official install script (get.docker.com)?" y; }; then
    local script
    script="$(mktemp)"
    download "https://get.docker.com" "$script"
    sh "$script"
    rm -f "$script"
    systemctl enable --now docker
    docker compose version >/dev/null 2>&1 || die "Docker installed, but 'docker compose' is still unavailable"
    info "Docker installed"
  else
    die "install Docker Engine with the Compose plugin (https://docs.docker.com/engine/install/), or re-run with --install-docker"
  fi
}

is_private_ipv4() {
  local ip="$1" a b
  IFS=. read -r a b _ _ <<< "$ip"
  [ "$a" = 10 ] && return 0
  [ "$a" = 192 ] && [ "$b" = 168 ] && return 0
  [ "$a" = 172 ] && [ "$b" -ge 16 ] && [ "$b" -le 31 ] && return 0
  [ "$a" = 100 ] && [ "$b" -ge 64 ] && [ "$b" -le 127 ] && return 0
  return 1
}

valid_ipv4() {
  local ip="$1" part
  [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || return 1
  IFS=. read -r -a parts <<< "$ip"
  for part in "${parts[@]}"; do [ "$part" -le 255 ] || return 1; done
}

network_of() {
  # network_of ADDRESS BITS -> network/bits
  local ip="$1" bits="$2" a b c d n mask
  IFS=. read -r a b c d <<< "$ip"
  n=$(( (a << 24) | (b << 16) | (c << 8) | d ))
  mask=$(( bits == 0 ? 0 : (0xFFFFFFFF << (32 - bits)) & 0xFFFFFFFF ))
  n=$(( n & mask ))
  printf '%d.%d.%d.%d/%d' $(( (n >> 24) & 255 )) $(( (n >> 16) & 255 )) $(( (n >> 8) & 255 )) $(( n & 255 )) "$bits"
}

detect_network() {
  step "Finding this server's network address"
  local detected_ip="" detected_cidr="" dev prefix
  if [ -f "$ENV_FILE" ] && [ "$RECONFIGURE" -eq 0 ]; then
    LAN_IP="${LAN_IP:-$(env_value HELIX_LAN_BIND_ADDRESS)}"
    LAN_CIDR="${LAN_CIDR:-$(env_value HELIX_ALLOWED_CLIENT_CIDR)}"
    PORT="${PORT:-$(env_value HELIX_LAN_PORT)}"
    if [ -n "$LAN_IP" ] && [ -n "$LAN_CIDR" ] && [ -n "$PORT" ]; then
      ip -4 -o addr show | awk '{print $4}' | cut -d/ -f1 | grep -qx "$LAN_IP"         || die "the saved address $LAN_IP is no longer on this server; re-run with --reconfigure"
      info "Keeping saved settings: http://$LAN_IP:$PORT for $LAN_CIDR (use --reconfigure to change)"
      return
    fi
  fi
  if [ -z "$LAN_IP" ]; then
    detected_ip="$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -n 1)"
    [ -n "$detected_ip" ] || detected_ip="$(ip -4 -o addr show scope global | awk '{print $4}' | cut -d/ -f1 | head -n 1)"
  fi
  local suggestion="${LAN_IP:-$detected_ip}"
  if [ -n "$suggestion" ] && ! is_private_ipv4 "$suggestion"; then
    warn "$suggestion is a public address. Helix is a private-LAN dashboard and will not listen on it."
    local private
    private="$(ip -4 -o addr show scope global | awk '{print $4}' | cut -d/ -f1 | while read -r candidate; do is_private_ipv4 "$candidate" && printf '%s\n' "$candidate"; done | head -n 1)"
    if [ -n "$private" ]; then
      info "Using this server's private address $private instead."
      suggestion="$private"
    elif [ -z "$LAN_IP" ]; then
      die "this server has no private network address. Join it to a private network (for example Tailscale, then pass --lan-ip with its 100.x address) and re-run."
    fi
  fi
  LAN_IP="$(prompt_value "Address other computers will use to open Helix" "$suggestion")"
  valid_ipv4 "$LAN_IP" || die "'$LAN_IP' is not an IPv4 address"
  is_private_ipv4 "$LAN_IP" || die "$LAN_IP is not a private address; Helix only listens on private networks"
  ip -4 -o addr show | awk '{print $4}' | cut -d/ -f1 | grep -qx "$LAN_IP" || die "$LAN_IP is not an address of this server"
  if [ -z "$LAN_CIDR" ]; then
    dev="$(ip -4 -o addr show | awk -v ip="$LAN_IP" '{split($4,a,"/"); if (a[1]==ip) print $2}' | head -n 1)"
    prefix="$(ip -4 -o addr show dev "$dev" | awk -v ip="$LAN_IP" '{split($4,a,"/"); if (a[1]==ip) print a[2]}' | head -n 1)"
    [ -n "$prefix" ] || prefix=24
    [ "$prefix" -lt 16 ] && prefix=16
    detected_cidr="$(network_of "$LAN_IP" "$prefix")"
    LAN_CIDR="$(prompt_value "Network allowed to open Helix" "$detected_cidr")"
  fi
  [[ "$LAN_CIDR" =~ ^[0-9.]+/[0-9]+$ ]] || die "'$LAN_CIDR' is not a network such as 192.168.1.0/24"
  [ "${LAN_CIDR#*/}" -ge 8 ] || die "$LAN_CIDR is too broad; use your LAN subnet"
  PORT="${PORT:-$(env_value HELIX_LAN_PORT 2>/dev/null || true)}"
  PORT="${PORT:-$DEFAULT_PORT}"
  if ! [[ "$PORT" =~ ^[0-9]+$ ]] || [ "$PORT" -lt 1024 ] || [ "$PORT" -gt 65535 ]; then
    die "port must be between 1024 and 65535"
  fi
  if ss -Hltn "sport = :$PORT" 2>/dev/null | grep -q . && ! docker ps --format '{{.Names}}' 2>/dev/null | grep -qx "$PROJECT"; then
    die "port $PORT is already in use on this server; choose another with --port"
  fi
  info "Helix will be at http://$LAN_IP:$PORT for computers on $LAN_CIDR"
}

# ---------------------------------------------------------------- setup --

ensure_groups() {
  step "Creating Helix system groups"
  local group
  for group in helix-broker helix-terminal; do
    if getent group "$group" >/dev/null; then info "$group exists"
    else groupadd --system "$group"; info "created $group"; fi
  done
  BROKER_GID="$(getent group helix-broker | cut -d: -f3)"
  TERMINAL_GID="$(getent group helix-terminal | cut -d: -f3)"
  [ "$BROKER_GID" != "$TERMINAL_GID" ] || die "helix-broker and helix-terminal must be different groups"
}

ensure_directories() {
  step "Preparing folders"
  install -d -m 0755 "$INSTALL_ROOT" /etc/helix
  install -d -m 0750 "$SRV_ROOT"
  install -d -m 0750 "$SRV_ROOT/instances" "$SRV_ROOT/backups"
  install -d -m 0700 "$SRV_ROOT/state" "$SRV_ROOT/state/instances" "$SRV_ROOT/state/network"
  local dir
  install -d -m 0750 "$SRV_ROOT/dashboard"
  # The dashboard container runs as $DASHBOARD_UID and needs its own private folders.
  for dir in "$SRV_ROOT/dashboard/data" "$SRV_ROOT/dashboard/backups"; do
    if [ ! -d "$dir" ]; then
      install -d -m 0700 -o "$DASHBOARD_UID" -g "$DASHBOARD_UID" "$dir"
      info "created $dir"
    fi
  done
  for dir in "${STORAGE_ROOTS[@]}"; do
    [ -d "$dir" ] || die "storage folder $dir does not exist"
    case "$(cd "$dir" && pwd)" in /|/etc|/usr|/boot|/proc|/sys|/dev|/run) die "$dir cannot be a Helix storage folder" ;; esac
  done
  info "Game servers: $SRV_ROOT/instances"
  info "Backups:      $SRV_ROOT/backups"
}

json_string_list() {
  local first=1 item
  printf '['
  for item in "$@"; do
    [ "$first" -eq 1 ] || printf ', '
    first=0
    printf '"%s"' "$(printf '%s' "$item" | sed 's/["\\]/\\&/g')"
  done
  printf ']'
}

write_configuration() {
  step "Writing configuration"
  local managed=("$SRV_ROOT/instances") roots dir
  for dir in "${STORAGE_ROOTS[@]}"; do managed+=("$(cd "$dir" && pwd)"); done
  if [ -f "$PRIVD_CONFIG" ] && [ "$RECONFIGURE" -eq 0 ]; then
    info "keeping $PRIVD_CONFIG (use --reconfigure to rewrite it)"
  else
    local amp=""
    [ -f /etc/helix/amp.json ] && amp='  "amp_credentials": "/etc/helix/amp.json",'
    roots="$(json_string_list "${managed[@]}")"
    umask 077
    {
      printf '{\n'
      printf '  "socket": "/run/helix/privd.sock",\n'
      [ -n "$amp" ] && printf '%s\n' "$amp"
      printf '  "managed_roots": %s,\n' "$roots"
      printf '  "analysis_roots": ["/"],\n'
      printf '  "host_control": { "docker_cleanup_state_root": "/var/lib/helix/docker-cleanup" },\n'
      printf '  "native": {\n'
      printf '    "state_root": "%s/state/instances",\n' "$SRV_ROOT"
      printf '    "instance_root": "%s/instances",\n' "$SRV_ROOT"
      printf '    "backup_root": "%s/backups",\n' "$SRV_ROOT"
      printf '    "docker_binary": "%s"\n' "$(command -v docker)"
      printf '  },\n'
      printf '  "network": { "state_root": "%s/state/network" }\n' "$SRV_ROOT"
      printf '}\n'
    } > "$PRIVD_CONFIG.new"
    chmod 0600 "$PRIVD_CONFIG.new"
    mv "$PRIVD_CONFIG.new" "$PRIVD_CONFIG"
    umask 022
    info "wrote $PRIVD_CONFIG"
  fi

  if [ -f "$ENV_FILE" ] && [ "$RECONFIGURE" -eq 0 ]; then
    info "keeping $ENV_FILE (use --reconfigure to rewrite it)"
    # The GIDs must always match this host's groups.
    sed -i "s/^HELIX_BROKER_GID=.*/HELIX_BROKER_GID=$BROKER_GID/; s/^HELIX_TERMINAL_GID=.*/HELIX_TERMINAL_GID=$TERMINAL_GID/" "$ENV_FILE"
  else
    umask 077
    cat > "$ENV_FILE.new" <<EOF
# Helix deployment settings, written by scripts/install.sh.
# Re-run the installer with --reconfigure to regenerate this file.
COMPOSE_PROJECT_NAME=$PROJECT
HELIX_LAN_BIND_ADDRESS=$LAN_IP
HELIX_LAN_PORT=$PORT
HELIX_BROKER_GID=$BROKER_GID
HELIX_TERMINAL_GID=$TERMINAL_GID
HELIX_GATEWAY_HOST=$LAN_IP
HELIX_BROWSER_ORIGIN=http://$LAN_IP:$PORT
HELIX_ALLOWED_CLIENT_CIDR=$LAN_CIDR
HELIX_SECONDARY_GATEWAY_HOST=unused.invalid
HELIX_SECONDARY_BROWSER_ORIGIN=http://unused.invalid
HELIX_SECONDARY_ALLOWED_CLIENT_CIDR=192.0.2.254/32
HELIX_DATA_DIR=$SRV_ROOT/dashboard/data
HELIX_BACKUP_DIR=$SRV_ROOT/dashboard/backups
EOF
    chmod 0600 "$ENV_FILE.new"
    mv "$ENV_FILE.new" "$ENV_FILE"
    umask 022
    info "wrote $ENV_FILE"
  fi

  # Extra storage roots must be writable inside the broker's sandbox.
  install -d -m 0755 "$UNIT_DIR/helix-privd.service.d"
  if [ "${#STORAGE_ROOTS[@]}" -gt 0 ]; then
    {
      printf '# Written by scripts/install.sh: folders Helix may manage.\n[Service]\n'
      for dir in "${managed[@]:1}"; do printf 'ReadWritePaths=-%s\n' "$dir"; done
    } > "$UNIT_DIR/helix-privd.service.d/storage.conf"
  fi
  compose config -q || die "the generated deployment settings are invalid"
}

build_helix() {
  local version revision
  version="$(source_version "$SOURCE_DIR")"
  [ -n "$version" ] || die "could not read the Helix version from $SOURCE_DIR/Cargo.toml"
  revision="$(git -C "$SOURCE_DIR" rev-parse --short=12 HEAD 2>/dev/null || printf 'release-%s' "$version")"
  export HELIX_IMAGE_TAG="$version" HELIX_SOURCE_REVISION="$revision"
  step "Building Helix $version (the first build takes several minutes)"
  compose build dashboard gateway
  local stage cid target
  stage="$(mktemp -d)"
  for target in privd terminald; do
    docker build --quiet --target "$target" --build-arg "HELIX_SOURCE_REVISION=$revision" -t "helix-$target-artifact:$version" "$SOURCE_DIR" >/dev/null
    cid="$(docker create "helix-$target-artifact:$version" "/helix-$target")"
    docker cp "$cid:/helix-$target" "$stage/helix-$target" >/dev/null
    docker rm -f "$cid" >/dev/null
  done
  install -d -m 0755 /usr/local/libexec
  if [ -f "$PRIVD_BIN" ] && ! cmp -s "$stage/helix-privd" "$PRIVD_BIN"; then cp -a "$PRIVD_BIN" "$PRIVD_BIN.previous"; fi
  install -m 0755 "$stage/helix-privd" "$PRIVD_BIN"
  install -m 0755 "$stage/helix-terminald" "$TERMINALD_BIN"
  rm -rf "$stage"
  info "Installed $PRIVD_BIN and $TERMINALD_BIN"
}

install_units() {
  step "Installing the host broker service"
  install -m 0644 "$SOURCE_DIR/deploy/helix-privd.service" "$UNIT_DIR/helix-privd.service"
  install -m 0644 "$SOURCE_DIR/deploy/helix-finalize-update.service" "$UNIT_DIR/helix-finalize-update.service"
  install -m 0644 "$SOURCE_DIR/deploy/helix-terminald@.service" "$UNIT_DIR/helix-terminald@.service"
  install -m 0644 "$SOURCE_DIR/deploy/helix-tmpfiles.conf" /etc/tmpfiles.d/helix.conf
  systemd-tmpfiles --create /etc/tmpfiles.d/helix.conf
  systemctl daemon-reload
  systemctl enable helix-privd.service >/dev/null 2>&1
  systemctl restart helix-privd.service
  local waited=0
  until [ -S /run/helix/privd.sock ]; do
    sleep 1
    waited=$((waited + 1))
    if [ "$waited" -ge 30 ] || systemctl is-failed --quiet helix-privd.service; then
      journalctl -u helix-privd --no-pager -n 15 >&2 || true
      die "the host broker did not start; see the log above"
    fi
  done
  info "helix-privd is running"
  if [ -n "$TERMINAL_USER" ]; then
    [ "$TERMINAL_USER" != root ] || die "the terminal cannot run as root"
    getent passwd "$TERMINAL_USER" >/dev/null || die "user $TERMINAL_USER does not exist"
    id -nG "$TERMINAL_USER" | tr ' ' '\n' | grep -qx helix-broker && die "$TERMINAL_USER must not be in the helix-broker group"
    systemctl enable --now "helix-terminald@$TERMINAL_USER.service" >/dev/null 2>&1
    info "terminal enabled for $TERMINAL_USER"
  fi
}

dashboard_exists() { docker ps -a --format '{{.Names}}' | grep -qx "$PROJECT"; }

issue_token() {
  # Prints the owner setup token, or nothing when an owner already exists.
  local output
  if output="$(compose run --rm --no-deps -T --entrypoint /app/bin/helixctl dashboard --config /app/config/helix.toml setup-token 2>&1)"; then
    printf '%s\n' "$output" | sed -n '2p'
  elif printf '%s' "$output" | grep -q "owner account already exists"; then
    printf ''
  else
    printf '%s\n' "$output" >&2
    die "could not create the owner setup token"
  fi
}

start_dashboard() {
  step "Starting the dashboard"
  TOKEN=""
  if ! dashboard_exists; then
    TOKEN="$(issue_token)"
  elif [ "$NEW_TOKEN" -eq 1 ]; then
    compose stop gateway dashboard >/dev/null 2>&1 || true
    TOKEN="$(issue_token)"
    [ -n "$TOKEN" ] || warn "an owner already exists, so no setup token is needed"
  fi
  compose up -d --wait dashboard gateway
  info "dashboard and gateway are healthy"
}

configure_firewall() {
  have ufw || return 0
  ufw status 2>/dev/null | grep -q "^Status: active" || return 0
  if ufw status | grep -q "$PORT/tcp.*ALLOW.*$LAN_CIDR"; then return 0; fi
  if [ "$OPEN_FIREWALL" = 1 ] || { [ "$ASSUME_YES" -eq 0 ] && confirm "UFW is active. Allow $LAN_CIDR to reach Helix on port $PORT?" y; }; then
    ufw allow from "$LAN_CIDR" to any port "$PORT" proto tcp comment "Helix dashboard" >/dev/null
    info "UFW now allows $LAN_CIDR to port $PORT"
  else
    warn "UFW is active and Helix's port is not allowed; other computers may not reach it"
  fi
}

verify_install() {
  step "Checking the dashboard responds"
  local code="" tries=0
  while [ "$tries" -lt 20 ]; do
    code="$(curl -s -o /dev/null -w '%{http_code}' -H "Host: $LAN_IP" "http://$LAN_IP:$PORT/healthz" 2>/dev/null || true)"
    [ "$code" = 204 ] && break
    sleep 2; tries=$((tries + 1))
  done
  [ "$code" = 204 ] || die "the dashboard did not answer at http://$LAN_IP:$PORT (status ${code:-none}); run '$0 --check'"
  info "http://$LAN_IP:$PORT is answering"
}

summary() {
  printf '\n%s%sHelix is running.%s\n\n' "$BOLD" "$GREEN" "$RESET"
  say "  Open ${BOLD}http://$LAN_IP:$PORT${RESET} from any computer on $LAN_CIDR."
  if [ -n "$TOKEN" ]; then
    if [ -n "$TOKEN_FILE" ]; then
      (umask 077; printf '%s\n' "$TOKEN" > "$TOKEN_FILE")
      say "  The one-time owner setup token was written to $TOKEN_FILE."
    else
      say "  One-time owner setup token: ${BOLD}$TOKEN${RESET}"
    fi
    say "  Paste it within 15 minutes, then create your login."
  fi
  say ""
  say "  Game servers:  $SRV_ROOT/instances"
  say "  Settings:      $ENV_FILE and $PRIVD_CONFIG"
  say "  Updates:       Host → Update Helix in the dashboard"
  say "  Diagnose:      sudo bash $SOURCE_DIR/scripts/install.sh --check"
  say ""
  say "  ${DIM}Helix is for private networks. To reach it from elsewhere, use Tailscale or a VPN.${RESET}"
}

# ---------------------------------------------------------------- modes --

# ok and bad always succeed, so "test && ok || bad" reads as if/else here.
# shellcheck disable=SC2015
run_check() {
  need_root
  local problems=0 name
  ok()  { printf '  %s✓%s %s\n' "$GREEN" "$RESET" "$*"; }
  bad() { printf '  %s✗%s %s\n' "$RED" "$RESET" "$*"; problems=$((problems + 1)); }
  say "${BOLD}Helix health check${RESET}"
  if have docker && docker info >/dev/null 2>&1; then ok "Docker is running"; else bad "Docker is not running (systemctl status docker)"; fi
  [ -r "$ENV_FILE" ] && ok "settings found at $ENV_FILE" || bad "no settings at $ENV_FILE; run the installer"
  [ -r "$PRIVD_CONFIG" ] && ok "broker configuration found" || bad "missing $PRIVD_CONFIG"
  if systemctl is-active --quiet helix-privd; then
    ok "helix-privd is running"
  else
    bad "helix-privd is not running ($(systemctl show helix-privd -p ActiveState --value 2>/dev/null || echo unknown), restarts: $(systemctl show helix-privd -p NRestarts --value 2>/dev/null || echo ?))"
    journalctl -u helix-privd --no-pager -n 5 -o cat 2>/dev/null | sed 's/^/      /' || true
  fi
  [ -S /run/helix/privd.sock ] && ok "broker socket is present" || bad "broker socket /run/helix/privd.sock is missing"
  for name in "$PROJECT" "$PROJECT-gateway"; do
    local health
    health="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$name" 2>/dev/null || echo missing)"
    [ "$health" = healthy ] && ok "$name is healthy" || bad "$name is $health"
  done
  local ip port
  ip="$(env_value HELIX_LAN_BIND_ADDRESS || true)"; port="$(env_value HELIX_LAN_PORT || true)"
  if [ -n "$ip" ]; then
    if ip -4 -o addr show | awk '{print $4}' | cut -d/ -f1 | grep -qx "$ip"; then ok "$ip is still an address of this server"
    else bad "$ip is no longer an address of this server; re-run the installer with --reconfigure"; fi
    local code
    code="$(curl -s -o /dev/null -w '%{http_code}' -H "Host: $ip" "http://$ip:$port/healthz" 2>/dev/null || true)"
    [ "$code" = 204 ] && ok "dashboard answers at http://$ip:$port" || bad "dashboard does not answer at http://$ip:$port (status ${code:-none})"
  fi
  if [ "$problems" -eq 0 ]; then say "${GREEN}Everything looks good.${RESET}"; return 0; fi
  say "${YELLOW}$problems problem(s) found.${RESET} Re-running the installer repairs most of them."
  return 1
}

run_uninstall() {
  need_root
  say "${BOLD}Removing Helix${RESET}"
  say "Game server containers (helix-game-*), their worlds, and backups are kept."
  if [ "$ASSUME_YES" -eq 0 ]; then
    confirm "Remove the Helix dashboard, gateway, broker, and services?" n || die "cancelled"
  fi
  if [ -r "$ENV_FILE" ] && have docker; then
    local compose_file
    compose_file="$(docker inspect -f '{{index .Config.Labels "com.docker.compose.project.config_files"}}' "$PROJECT" 2>/dev/null || true)"
    if [ -n "$compose_file" ] && [ -f "${compose_file%%,*}" ]; then
      docker compose --env-file "$ENV_FILE" --project-name "$PROJECT" -f "${compose_file%%,*}" down >/dev/null 2>&1 || true
    fi
  fi
  docker rm -f "$PROJECT-gateway" "$PROJECT" >/dev/null 2>&1 || true
  local unit
  for unit in $(systemctl list-units --all --plain --no-legend 'helix-terminald@*' 2>/dev/null | awk '{print $1}'); do
    systemctl disable --now "$unit" >/dev/null 2>&1 || true
  done
  systemctl disable --now helix-privd.service >/dev/null 2>&1 || true
  rm -f "$UNIT_DIR/helix-privd.service" "$UNIT_DIR/helix-finalize-update.service" "$UNIT_DIR/helix-terminald@.service" /etc/tmpfiles.d/helix.conf
  rm -rf "$UNIT_DIR/helix-privd.service.d"
  rm -f "$PRIVD_BIN" "$PRIVD_BIN.previous" "$TERMINALD_BIN"
  systemctl daemon-reload
  say "Removed the Helix services and binaries."
  if [ "$PURGE" -eq 1 ]; then
    warn "--purge deletes $SRV_ROOT (game servers, worlds, and backups), $INSTALL_ROOT, and /etc/helix."
    if [ "$ASSUME_YES" -eq 1 ] || [ -z "$TTY" ]; then die "--purge needs an interactive confirmation"; fi
    printf 'Type DELETE to permanently delete them: ' > "$TTY"
    local answer
    read -r answer < "$TTY" || answer=""
    [ "$answer" = DELETE ] || die "purge cancelled; data kept"
    docker ps -a --format '{{.Names}}' | grep '^helix-game-' | xargs -r docker rm -f >/dev/null 2>&1 || true
    rm -rf "$SRV_ROOT" "$INSTALL_ROOT" /etc/helix /var/lib/helix
    say "Purged Helix data."
  else
    say "Kept $SRV_ROOT, $INSTALL_ROOT, and /etc/helix. Reinstall any time to pick them up again."
  fi
}

main() {
  case "$MODE" in
    check) run_check; exit $? ;;
    uninstall) run_uninstall; exit 0 ;;
  esac
  printf '%sHelix installer%s\n' "$BOLD" "$RESET"
  preflight
  resolve_source
  info "Source: $SOURCE_DIR ($(source_version "$SOURCE_DIR"))"
  ensure_docker
  detect_network
  ensure_groups
  ensure_directories
  write_configuration
  build_helix
  install_units
  start_dashboard
  configure_firewall
  verify_install
  summary
}

main
