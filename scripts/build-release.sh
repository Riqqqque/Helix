#!/usr/bin/env bash
set -Eeuo pipefail

umask 022

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(CDPATH='' cd -- "$script_dir/.." && pwd -P)"
cd -- "$repo_root" || exit 1

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

for required_command in basename cargo chmod cp find install mkdir mountpoint node npm realpath rm rustc sed sha256sum sort tar uname xargs; do
  command -v "$required_command" >/dev/null 2>&1 ||
    fail "required build command is unavailable: $required_command"
done

if [[ "$(uname -s)" != "Linux" ]]; then
  fail "Linux release bundles must be built on Linux; use build-release.ps1 from Windows with WSL"
fi

host_target="$(rustc -vV | sed -n 's/^host: //p')"
[[ -n "$host_target" ]] || fail "could not determine the Rust host target"
[[ "$host_target" =~ ^[A-Za-z0-9._+-]+-unknown-linux-[A-Za-z0-9._+-]+$ ]] ||
  fail "unsupported release host target: $host_target"

target_root="$repo_root/target"
if [[ -e "$target_root" || -L "$target_root" ]]; then
  [[ -d "$target_root" && ! -L "$target_root" ]] ||
    fail "Cargo target root is not a safe directory: $target_root"
else
  mkdir -- "$target_root"
fi
[[ "$(realpath -e -- "$target_root")" == "$target_root" ]] ||
  fail "Cargo target root resolves outside its exact repository path"
export CARGO_TARGET_DIR="$target_root"

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

printf 'Checking Rust workspace...\n'
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features --target "$host_target" -- -D warnings
cargo test --locked --workspace --all-targets --all-features --target "$host_target"
cargo build --locked --release --workspace --target "$host_target"

printf 'Checking frontend workspace...\n'
(
  cd -- "$repo_root/frontend"
  npm ci --no-audit --no-fund
  npm run check
  npm audit --audit-level=moderate
)

binary_root="$target_root/$host_target/release"
for binary in helixd helixctl; do
  binary_path="$binary_root/$binary"
  [[ -f "$binary_path" && ! -L "$binary_path" ]] ||
    fail "release build did not produce a safe $binary binary for $host_target"
  [[ "$("$binary_path" --version)" == "$binary $version" ]] ||
    fail "release $binary version does not match package metadata"
done

release_root="$target_root/helix-release"
bundle_name="helix-$version-$host_target"
bundle_dir="$release_root/$bundle_name"
archive_path="$release_root/$bundle_name.tar.gz"

case "$bundle_dir" in
  "$repo_root"/target/helix-release/helix-*) ;;
  *) fail "refusing to replace an unexpected release path: $bundle_dir" ;;
esac

mkdir -p -- "$release_root"
[[ -d "$release_root" && ! -L "$release_root" ]] ||
  fail "release output root is not a safe directory: $release_root"
if [[ -e "$bundle_dir" || -L "$bundle_dir" ]]; then
  [[ -d "$bundle_dir" && ! -L "$bundle_dir" ]] ||
    fail "refusing to replace an unsafe release bundle path: $bundle_dir"
  ! mountpoint -q "$bundle_dir" ||
    fail "refusing to replace a mounted release bundle path: $bundle_dir"
  find "$bundle_dir" -xdev -depth -delete
fi
rm -f -- "$archive_path" "$archive_path.sha256"

install -d -m 0755 \
  "$bundle_dir/bin" \
  "$bundle_dir/web" \
  "$bundle_dir/packaging/systemd" \
  "$bundle_dir/packaging/sysusers" \
  "$bundle_dir/packaging/tmpfiles"
install -m 0755 "$binary_root/helixd" "$bundle_dir/bin/helixd"
install -m 0755 "$binary_root/helixctl" "$bundle_dir/bin/helixctl"
cp -a -- "$repo_root/frontend/dist/." "$bundle_dir/web/"
find "$bundle_dir/web" -type d -exec chmod 0755 {} +
find "$bundle_dir/web" -type f -exec chmod 0644 {} +
install -m 0644 \
  "$repo_root/packaging/systemd/helixd.service" \
  "$bundle_dir/packaging/systemd/helixd.service"
install -m 0644 \
  "$repo_root/packaging/sysusers/helix.conf" \
  "$bundle_dir/packaging/sysusers/helix.conf"
install -m 0644 \
  "$repo_root/packaging/tmpfiles/helix.conf" \
  "$bundle_dir/packaging/tmpfiles/helix.conf"
install -m 0644 "$repo_root/packaging/helix.toml" "$bundle_dir/packaging/helix.toml"
install -m 0644 "$repo_root/scripts/package-common.sh" "$bundle_dir/package-common.sh"
install -m 0755 "$repo_root/scripts/install-local.sh" "$bundle_dir/install-local.sh"
install -m 0755 "$repo_root/scripts/rollback-local.sh" "$bundle_dir/rollback-local.sh"
install -m 0755 "$repo_root/scripts/uninstall-local.sh" "$bundle_dir/uninstall-local.sh"
install -m 0644 "$repo_root/LICENSE" "$bundle_dir/LICENSE"

unsafe_bundle_entry="$(find "$bundle_dir" -xdev ! -type d ! -type f -print -quit)"
[[ -z "$unsafe_bundle_entry" ]] ||
  fail "release bundle contains a symlink or special entry: $unsafe_bundle_entry"
while IFS= read -r -d '' bundle_entry; do
  relative_entry="./${bundle_entry#"$bundle_dir"/}"
  [[ "$relative_entry" =~ ^\./[A-Za-z0-9._/+@-]+$ ]] ||
    fail "release bundle contains an unsupported path: $relative_entry"
  if [[ -d "$bundle_entry" ]]; then
    ! mountpoint -q "$bundle_entry" ||
      fail "release bundle contains a nested mount point: $relative_entry"
  fi
done < <(find "$bundle_dir" -xdev -mindepth 1 -print0)

(
  cd -- "$bundle_dir"
  find . -type f ! -path ./SHA256SUMS -print0 |
    LC_ALL=C sort -z |
    xargs -0 sha256sum > SHA256SUMS
  sha256sum --check --strict SHA256SUMS >/dev/null
)

tar -C "$release_root" -czf "$archive_path" "$bundle_name"
(
  cd -- "$release_root"
  sha256sum "$(basename -- "$archive_path")" > "$(basename -- "$archive_path").sha256"
)

printf '\nBuilt unsigned local release artifacts:\n'
printf '  bundle:  %s\n' "$bundle_dir"
printf '  archive: %s\n' "$archive_path"
printf '  checksum: %s\n' "$archive_path.sha256"
printf '  target:  %s\n' "$host_target"
