#!/usr/bin/env bash
set -Eeuo pipefail

umask 022

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(CDPATH='' cd -- "$script_dir/.." && pwd -P)"
cd -- "$repo_root" || exit 1

start_service=1
install_deps=0

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: ./scripts/install-from-source.sh [--no-start] [--install-deps]

Build Helix from this checkout and install helixd as a systemd service.

Run as a normal user on 64-bit systemd Linux (x86_64 or aarch64). The script
asks for sudo only to install files, or to install missing compiler packages
when you pass --install-deps. It does not install helix-privd, so host, file,
firewall, package, and native game-server controls stay unavailable until you
follow docs/CONTAINER-DEPLOYMENT.md.

There is no signed download. This is an unsigned source build of the scoped
local package, not a supported production installer.
USAGE
}

os_release_field() {
  local key=$1
  local file=/etc/os-release
  [[ -f "$file" && ! -L "$file" ]] || return 1
  awk -F= -v key="$key" '
    $1 == key {
      value = substr($0, index($0, "=") + 1)
      if (value ~ /^".*"$/) {
        value = substr(value, 2, length(value) - 2)
      }
      print value
      exit
    }
  ' "$file"
}

pkg_family() {
  local id like
  id="$(os_release_field ID 2>/dev/null || true)"
  like="$(os_release_field ID_LIKE 2>/dev/null || true)"
  id=${id,,}
  like=${like,,}
  case "$id" in
    ubuntu|debian|linuxmint|pop|elementary|raspbian|raspberrypi|zorin|kali|devuan|neon)
      printf 'debian'
      return 0
      ;;
    fedora|rhel|centos|rocky|almalinux|ol|amzn|nobara)
      printf 'fedora'
      return 0
      ;;
    opensuse-leap|opensuse-tumbleweed|opensuse|sles|sled)
      printf 'suse'
      return 0
      ;;
    arch|manjaro|endeavouros|garuda|cachyos|archcraft)
      printf 'arch'
      return 0
      ;;
    alpine)
      printf 'alpine'
      return 0
      ;;
  esac
  case " $like " in
    *" debian "*|*" ubuntu "*) printf 'debian'; return 0 ;;
    *" fedora "*|*" rhel "*|*" centos "*) printf 'fedora'; return 0 ;;
    *" suse "*) printf 'suse'; return 0 ;;
    *" arch "*) printf 'arch'; return 0 ;;
  esac
  printf 'unknown'
}

print_rust_and_node() {
  cat <<'EOF'
Rust 1.88 or newer (rustup, not a distro rustc that is too old):

  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0
  . "$HOME/.cargo/env"

Node.js 22.12 or newer and npm. Distro packages are fine when they meet that
floor; otherwise use nvm, fnm, or the Node.js Linux binaries.
EOF
}

print_distro_packages() {
  local family=$1
  cat <<EOF
Compiler toolchain for this host ($family):
EOF
  case "$family" in
    debian)
      cat <<'EOF'
  sudo apt-get update
  sudo apt-get install -y build-essential pkg-config git curl ca-certificates
EOF
      ;;
    fedora)
      cat <<'EOF'
  sudo dnf install -y gcc gcc-c++ make pkgconf-pkg-config git curl ca-certificates
EOF
      ;;
    suse)
      cat <<'EOF'
  sudo zypper install -y gcc gcc-c++ make pkg-config git curl ca-certificates
EOF
      ;;
    arch)
      cat <<'EOF'
  sudo pacman -S --needed --noconfirm base-devel pkgconf git curl ca-certificates
EOF
      ;;
    alpine)
      cat <<'EOF'
  sudo apk add build-base pkgconf git curl ca-certificates
  Alpine's default OpenRC image is not enough; helixd needs systemd as PID 1.
EOF
      ;;
    *)
      cat <<'EOF'
  Install a C compiler, pkg-config, git, curl, and CA certificates with this
  distribution's package manager. GNU coreutils (sha256sum, install, realpath)
  and util-linux (flock) are required for the local package scripts.
EOF
      ;;
  esac
}

print_prereqs() {
  local family
  family="$(pkg_family)"
  printf '\nInstall the build tools, then re-run ./scripts/install-from-source.sh\n\n' >&2
  print_distro_packages "$family" >&2
  printf '\n' >&2
  print_rust_and_node >&2
  printf '\nOr pass --install-deps to install only the compiler packages above.\n' >&2
}

install_distro_packages() {
  local family
  family="$(pkg_family)"
  printf 'Installing compiler packages for %s with sudo...\n' "$family"
  case "$family" in
    debian)
      sudo -- apt-get update
      sudo -- apt-get install -y build-essential pkg-config git curl ca-certificates
      ;;
    fedora)
      sudo -- dnf install -y gcc gcc-c++ make pkgconf-pkg-config git curl ca-certificates
      ;;
    suse)
      sudo -- zypper install -y gcc gcc-c++ make pkg-config git curl ca-certificates
      ;;
    arch)
      sudo -- pacman -S --needed --noconfirm base-devel pkgconf git curl ca-certificates
      ;;
    alpine)
      sudo -- apk add build-base pkgconf git curl ca-certificates
      ;;
    *)
      fail "this host's package manager is not one of apt, dnf, zypper, pacman, or apk; install gcc, pkg-config, git, and curl by hand"
      ;;
  esac
}

while (($# > 0)); do
  case "$1" in
    --no-start)
      start_service=0
      shift
      ;;
    --install-deps)
      install_deps=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done

[[ "$(uname -s)" == "Linux" ]] ||
  fail "this installer targets Linux/systemd; on Windows use WSL or a loopback cargo build"
[[ "$(id -u)" -ne 0 ]] ||
  fail "run as a normal user; the script will ask for sudo only to install files"
[[ -d /run/systemd/system ]] || fail "systemd is not the active service manager"

machine="$(uname -m)"
case "$machine" in
  x86_64|aarch64|arm64) ;;
  *) fail "the local package is built for 64-bit x86_64 and aarch64 (found $machine)" ;;
esac

if [[ ! -e /sys/fs/cgroup/cgroup.controllers ]]; then
  printf 'warning: cgroup v2 is not visible; host resource views may be incomplete.\n' >&2
fi

if ((install_deps == 1)); then
  install_distro_packages
fi

missing=()
for required_command in rustc cargo node npm git sudo pkg-config; do
  command -v "$required_command" >/dev/null 2>&1 || missing+=("$required_command")
done
if ! command -v cc >/dev/null 2>&1 &&
  ! command -v gcc >/dev/null 2>&1 &&
  ! command -v clang >/dev/null 2>&1; then
  missing+=("c-compiler")
fi
if ! command -v sha256sum >/dev/null 2>&1 ||
  ! sha256sum --help >/dev/null 2>&1; then
  missing+=("gnu-sha256sum")
fi
if ((${#missing[@]} > 0)); then
  printf 'error: missing build tools: %s\n' "${missing[*]}" >&2
  print_prereqs
  exit 1
fi

rust_version="$(rustc --version | awk '{print $2}')"
[[ "$rust_version" =~ ^([0-9]+)\.([0-9]+) ]] ||
  fail "could not parse rustc version: $rust_version"
rust_major="${BASH_REMATCH[1]}"
rust_minor="${BASH_REMATCH[2]}"
if ((rust_major < 1)) || ((rust_major == 1 && rust_minor < 88)); then
  printf 'error: Rust 1.88 or newer is required (found rustc %s)\n' "$rust_version" >&2
  print_rust_and_node >&2
  exit 1
fi

if ! node --input-type=commonjs -e '
      const [major, minor] = process.versions.node.split(".").map(Number);
      if (!(major > 22 || (major === 22 && minor >= 12))) process.exit(1);
    '; then
  printf 'error: Node.js 22.12 or newer is required (found %s)\n' "$(node --version)" >&2
  print_rust_and_node >&2
  exit 1
fi

host_target="$(rustc -vV | sed -n 's/^host: //p')"
if [[ "$host_target" == *linux-musl* ]]; then
  printf 'warning: musl targets are untested. Prefer a glibc distro (Debian, Fedora, Arch, openSUSE).\n' >&2
fi

printf 'Building Helix from this checkout, then installing helixd with sudo.\n'
printf 'First compile can take a while. Host and game controls are not included.\n\n'

"$script_dir/build-release.sh"

version="$(
  cargo metadata --locked --no-deps --format-version 1 |
    node --input-type=commonjs -e '
      const fs = require("node:fs");
      const metadata = JSON.parse(fs.readFileSync(0, "utf8"));
      const package = metadata.packages.find((candidate) => candidate.name === "helixd");
      if (!package) process.exit(2);
      process.stdout.write(package.version);
    '
)"
[[ "$version" =~ ^[A-Za-z0-9.+-]+$ ]] || fail "could not determine a safe Helix version"

bundle_dir="$repo_root/target/helix-release/helix-$version-$host_target"
[[ -d "$bundle_dir" && ! -L "$bundle_dir" && -x "$bundle_dir/bin/helixd" ]] ||
  fail "release build did not produce a safe bundle at $bundle_dir"

printf '\nInstalling helixd with sudo...\n'
if [[ "$start_service" -eq 0 ]]; then
  sudo -- "$bundle_dir/install-local.sh" --no-start
else
  sudo -- "$bundle_dir/install-local.sh"
fi

cat <<'EOF'

helixd is installed on 127.0.0.1:8080.

Create the owner from this host:

  sudo -u helix -- helixctl --config /etc/helix/helix.toml setup-token

Then open http://127.0.0.1:8080 and paste the token within 15 minutes.

This local package is the dashboard only. Host files, UFW or firewalld, APT or
DNF updates, and native game servers need helix-privd. Selected package updates
and one-click Tailscale/Jellyfin installs still require a Debian-family APT
host. See docs/CONTAINER-DEPLOYMENT.md.
EOF
