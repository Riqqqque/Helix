#!/usr/bin/env bash
set -Eeuo pipefail

umask 022

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(CDPATH='' cd -- "$script_dir/.." && pwd -P)"
cd -- "$repo_root" || exit 1

start_service=1

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: ./scripts/install-from-source.sh [--no-start]

Build Helix from this checkout and install helixd as a systemd service.

Run as a normal user on 64-bit Ubuntu 24.04 (or similar systemd Linux). The
script asks for sudo only when it installs files. It does not install
helix-privd, so host, file, firewall, package, and native game-server controls
stay unavailable until you follow docs/CONTAINER-DEPLOYMENT.md.

There is no signed download. This is an unsigned source build of the scoped
local package, not a supported production installer.
USAGE
}

print_ubuntu_prereqs() {
  cat <<'EOF'

Install the build tools, then re-run ./scripts/install-from-source.sh

  sudo apt-get update
  sudo apt-get install -y build-essential pkg-config git curl ca-certificates

  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0
  . "$HOME/.cargo/env"

  # Node.js 22.12 or newer. NodeSource is one option; any 22.12+ install is fine.
  curl -fsSL https://deb.nodesource.com/setup_22.x | sudo bash -
  sudo apt-get install -y nodejs

EOF
}

while (($# > 0)); do
  case "$1" in
    --no-start)
      start_service=0
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
  fail "this installer targets Linux/systemd; on Windows use WSL Ubuntu or a loopback cargo build"
[[ "$(id -u)" -ne 0 ]] ||
  fail "run as a normal user; the script will ask for sudo only to install files"
[[ -d /run/systemd/system ]] || fail "systemd is not the active service manager"

missing=()
for required_command in rustc cargo node npm git sudo pkg-config; do
  command -v "$required_command" >/dev/null 2>&1 || missing+=("$required_command")
done
if ! command -v cc >/dev/null 2>&1 &&
  ! command -v gcc >/dev/null 2>&1 &&
  ! command -v clang >/dev/null 2>&1; then
  missing+=("c-compiler")
fi
if ((${#missing[@]} > 0)); then
  printf 'error: missing build tools: %s\n' "${missing[*]}" >&2
  print_ubuntu_prereqs >&2
  exit 1
fi

rust_version="$(rustc --version | awk '{print $2}')"
[[ "$rust_version" =~ ^([0-9]+)\.([0-9]+) ]] ||
  fail "could not parse rustc version: $rust_version"
rust_major="${BASH_REMATCH[1]}"
rust_minor="${BASH_REMATCH[2]}"
if ((rust_major < 1)) || ((rust_major == 1 && rust_minor < 88)); then
  printf 'error: Rust 1.88 or newer is required (found rustc %s)\n' "$rust_version" >&2
  print_ubuntu_prereqs >&2
  exit 1
fi

if ! node --input-type=commonjs -e '
      const [major, minor] = process.versions.node.split(".").map(Number);
      if (!(major > 22 || (major === 22 && minor >= 12))) process.exit(1);
    '; then
  printf 'error: Node.js 22.12 or newer is required (found %s)\n' "$(node --version)" >&2
  print_ubuntu_prereqs >&2
  exit 1
fi

printf 'Building Helix from this checkout, then installing helixd with sudo.\n'
printf 'First compile can take a while. Host and game controls are not included.\n\n'

"$script_dir/build-release.sh"

host_target="$(rustc -vV | sed -n 's/^host: //p')"
[[ -n "$host_target" ]] || fail "could not determine the Rust host target"
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

This local package is the dashboard only. Host files, UFW, packages, and native
game servers need helix-privd. See docs/CONTAINER-DEPLOYMENT.md.
EOF
