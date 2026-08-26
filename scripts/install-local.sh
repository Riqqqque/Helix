#!/usr/bin/env bash
set -Eeuo pipefail

umask 077

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
bundle_dir=""
start_service=1

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: sudo ./install-local.sh [--bundle DIRECTORY] [--no-start]

Install or upgrade Helix from a locally extracted, checksummed release bundle.
The package-file transaction does not modify an existing Helix configuration or
roll back application databases.
USAGE
}

while (($# > 0)); do
  case "$1" in
    --bundle)
      (($# >= 2)) || fail "--bundle requires a directory"
      bundle_dir=$2
      shift 2
      ;;
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

for required_command in \
  bash chmod chown cmp cp cut date env find flock id install mktemp mountpoint mv realpath rm \
  rmdir runuser sha256sum sort stat systemctl systemd-sysusers systemd-tmpfiles uname \
  xargs; do
  command -v "$required_command" >/dev/null 2>&1 ||
    fail "required install command is unavailable: $required_command"
done

[[ "$(uname -s)" == "Linux" ]] || fail "the Helix local installer supports Linux/systemd hosts only"
[[ "$(id -u)" -eq 0 ]] || fail "run this installer as root, for example with sudo"
[[ -d /run/systemd/system ]] || fail "systemd is not the active service manager"

if [[ -z "$bundle_dir" ]]; then
  if [[ -x "$script_dir/bin/helixd" && -d "$script_dir/packaging" ]]; then
    bundle_dir=$script_dir
  else
    fail "pass the extracted release bundle directory with --bundle"
  fi
fi

[[ -d "$bundle_dir" && ! -L "$bundle_dir" ]] || fail "bundle path is missing or unsafe: $bundle_dir"
bundle_dir="$(realpath -e -- "$bundle_dir")"

required_files=(
  bin/helixd
  bin/helixctl
  web/index.html
  packaging/systemd/helixd.service
  packaging/sysusers/helix.conf
  packaging/tmpfiles/helix.conf
  packaging/helix.toml
  package-common.sh
  install-local.sh
  rollback-local.sh
  uninstall-local.sh
  SHA256SUMS
)
for relative_path in "${required_files[@]}"; do
  full_path="$bundle_dir/$relative_path"
  [[ -f "$full_path" && ! -L "$full_path" ]] ||
    fail "bundle file is missing or unsafe: $relative_path"
done
[[ -x "$bundle_dir/bin/helixd" ]] || fail "bundle helixd is not executable"
[[ -x "$bundle_dir/bin/helixctl" ]] || fail "bundle helixctl is not executable"
for lifecycle_script in install-local.sh rollback-local.sh uninstall-local.sh; do
  [[ -x "$bundle_dir/$lifecycle_script" ]] ||
    fail "bundle lifecycle script is not executable: $lifecycle_script"
done

unsafe_bundle_entry="$(
  find "$bundle_dir" -xdev \
    \( -type l -o -type b -o -type c -o -type p -o -type s \) \
    -print -quit
)"
[[ -z "$unsafe_bundle_entry" ]] ||
  fail "bundle contains a symlink or special entry: $unsafe_bundle_entry"

declare -A manifest_paths=()
declare -A manifest_hashes=()
manifest_entries=0
while IFS= read -r manifest_line; do
  [[ "$manifest_line" =~ ^[0-9a-f]{64}\ \ \./[A-Za-z0-9._/+@-]+$ ]] ||
    fail "bundle checksum manifest contains an unsafe entry"
  manifest_path=${manifest_line:66}
  [[ -z "${manifest_paths[$manifest_path]+present}" ]] ||
    fail "bundle checksum manifest contains a duplicate path"
  case "/$manifest_path/" in
    *"/../"*|*"//"*) fail "bundle checksum manifest escapes the bundle root" ;;
  esac
  manifest_file="$bundle_dir/${manifest_path#./}"
  [[ -f "$manifest_file" && ! -L "$manifest_file" ]] ||
    fail "bundle checksum manifest references a missing or unsafe file"
  resolved_manifest_file="$(realpath -e -- "$manifest_file")"
  case "$resolved_manifest_file" in
    "$bundle_dir"/*) ;;
    *) fail "bundle checksum manifest resolves outside the bundle root" ;;
  esac
  manifest_paths["$manifest_path"]=1
  manifest_hashes["$manifest_path"]=${manifest_line:0:64}
  ((manifest_entries += 1))
done < "$bundle_dir/SHA256SUMS"
((manifest_entries > 0)) || fail "bundle checksum manifest is empty"

while IFS= read -r -d '' bundle_file; do
  relative_file="./${bundle_file#"$bundle_dir"/}"
  [[ -n "${manifest_paths[$relative_file]+present}" ]] ||
    fail "bundle contains an unchecksummed file: $relative_file"
done < <(find "$bundle_dir" -xdev -type f ! -path "$bundle_dir/SHA256SUMS" -print0)

(
  cd -- "$bundle_dir"
  sha256sum --check --strict SHA256SUMS
) >/dev/null || fail "bundle checksum verification failed"

# Source only the helper that was just covered by the exact bundle checksum
# manifest. The release bundle remains unsigned; this verifies integrity, not
# publisher authenticity.
# shellcheck source=scripts/package-common.sh
source "$bundle_dir/package-common.sh"

verify_payload_file() {
  local manifest_key=$1
  local file=$2
  local expected=${manifest_hashes[$manifest_key]:-}
  local actual

  [[ -n "$expected" ]] || fail "bundle payload is missing from its checksum manifest: $manifest_key"
  [[ -f "$file" && ! -L "$file" ]] || fail "verified payload file is missing or unsafe: $file"
  actual="$(sha256sum -- "$file" | cut -d ' ' -f 1)"
  [[ "$actual" == "$expected" ]] || fail "staged payload checksum changed: $manifest_key"
}

verify_payload_tree() {
  local manifest_prefix=$1
  local tree=$2
  local tree_file
  local manifest_key
  local relative_path

  while IFS= read -r -d '' tree_file; do
    relative_path=${tree_file#"$tree"/}
    manifest_key="$manifest_prefix/$relative_path"
    verify_payload_file "$manifest_key" "$tree_file"
  done < <(find "$tree" -xdev -type f -print0)

  for manifest_key in "${!manifest_hashes[@]}"; do
    case "$manifest_key" in
      "$manifest_prefix"/*)
        relative_path=${manifest_key#"$manifest_prefix"/}
        [[ -f "$tree/$relative_path" && ! -L "$tree/$relative_path" ]] ||
          fail "staged payload is incomplete: $manifest_key"
        ;;
    esac
  done
}

helix_require_linux_root
helix_acquire_package_lock
helix_assert_system_paths_safe

critical_state_database=/var/lib/helix/state/helix-state.db
critical_state_existed_before=0
for state_artifact in \
  "$critical_state_database" \
  "$critical_state_database-wal" \
  "$critical_state_database-shm"; do
  [[ ! -L "$state_artifact" ]] || fail "critical state database artifact is a symlink: $state_artifact"
done
if [[ -e "$critical_state_database" ]]; then
  [[ -f "$critical_state_database" ]] || fail "critical state database is not a regular file"
  critical_state_existed_before=1
elif [[ -e "$critical_state_database-wal" || -e "$critical_state_database-shm" ]]; then
  fail "orphaned critical state database sidecar exists without helix-state.db"
fi

helix_prepare_metadata_roots
helix_prepare_package_parents

rollback_armed=0
transaction_snapshot_id=""
bootstrap_attempted=0
config_staging=""

cleanup_config_staging() {
  if [[ -z "$config_staging" ]]; then
    return 0
  fi
  case "$config_staging" in
    /etc/helix/.helix.toml.helix-new.*) ;;
    *) return 1 ;;
  esac
  if [[ -e "$config_staging" || -L "$config_staging" ]]; then
    [[ -f "$config_staging" && ! -L "$config_staging" ]] || return 1
    [[ "$(stat -c '%u' -- "$config_staging")" == "0" ]] || return 1
    rm -f -- "$config_staging"
  fi
  config_staging=""
}

finish_install() {
  local status=$?
  local restore_status=0
  local cleanup_status=0

  trap - EXIT HUP INT TERM
  set +e
  if ((rollback_armed == 1)); then
    printf 'Install failed after helixd was stopped; restoring package snapshot %s.\n' \
      "$transaction_snapshot_id" >&2
    (
      set -Eeuo pipefail
      helix_restore_package_snapshot "$transaction_snapshot_id"
    )
    restore_status=$?
    if ((restore_status == 0)); then
      printf 'Previous package files and service state were restored.\n' >&2
    else
      printf 'error: automatic package rollback failed; snapshot retained: %s/%s\n' \
        "$HELIX_ROLLBACK_ROOT" "$transaction_snapshot_id" >&2
      status=1
    fi
  fi
  if ((bootstrap_attempted == 1)); then
    printf 'Setup state is preserved. If no owner was created, replace the token only while helixd is stopped.\n' >&2
  fi
  (
    set -Eeuo pipefail
    cleanup_config_staging
    helix_cleanup_transients
  )
  cleanup_status=$?
  if ((cleanup_status != 0)); then
    printf 'warning: one or more validated package staging paths could not be cleaned\n' >&2
    status=1
  fi
  exit "$status"
}

trap finish_install EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

for target in "${HELIX_MANAGED_TARGETS[@]}"; do
  helix_validate_existing_managed_target "$target"
done

helix_stage_file "$bundle_dir/bin/helixd" /usr/bin/helixd 0755
helix_stage_file "$bundle_dir/bin/helixctl" /usr/bin/helixctl 0755
helix_stage_tree "$bundle_dir/web" /usr/share/helix/web
helix_stage_file \
  "$bundle_dir/packaging/systemd/helixd.service" \
  /usr/lib/systemd/system/helixd.service \
  0644
helix_stage_file \
  "$bundle_dir/packaging/sysusers/helix.conf" \
  /usr/lib/sysusers.d/helix.conf \
  0644
helix_stage_file \
  "$bundle_dir/packaging/tmpfiles/helix.conf" \
  /usr/lib/tmpfiles.d/helix.conf \
  0644

declare -A managed_payload_keys=(
  [/usr/bin/helixd]=./bin/helixd
  [/usr/bin/helixctl]=./bin/helixctl
  [/usr/lib/systemd/system/helixd.service]=./packaging/systemd/helixd.service
  [/usr/lib/sysusers.d/helix.conf]=./packaging/sysusers/helix.conf
  [/usr/lib/tmpfiles.d/helix.conf]=./packaging/tmpfiles/helix.conf
)
for target in "${!managed_payload_keys[@]}"; do
  verify_payload_file "${managed_payload_keys[$target]}" "${HELIX_STAGED_PATHS[$target]}"
done
verify_payload_tree ./web "${HELIX_STAGED_PATHS[/usr/share/helix/web]}"

"${HELIX_STAGED_PATHS[/usr/bin/helixd]}" --version
"${HELIX_STAGED_PATHS[/usr/bin/helixctl]}" --version

# Account, private data roots, and initial configuration are deliberately not
# part of the package-file rollback. They are created before the service stop
# and are preserved by rollback and uninstall.
systemd-sysusers "${HELIX_STAGED_PATHS[/usr/lib/sysusers.d/helix.conf]}"
id helix >/dev/null 2>&1 || fail "the helix service account was not created"
systemd-tmpfiles --create "${HELIX_STAGED_PATHS[/usr/lib/tmpfiles.d/helix.conf]}"

if [[ -e /etc/helix || -L /etc/helix ]]; then
  [[ -d /etc/helix && ! -L /etc/helix ]] || fail "/etc/helix is not a safe directory"
  [[ "$(stat -c '%u' -- /etc/helix)" == "0" ]] || fail "/etc/helix must be owned by root"
  helix_assert_not_service_writable /etc/helix
else
  install -d -o root -g helix -m 0750 -- /etc/helix
fi

if [[ -e /etc/helix/helix.toml || -L /etc/helix/helix.toml ]]; then
  [[ -f /etc/helix/helix.toml && ! -L /etc/helix/helix.toml ]] ||
    fail "/etc/helix/helix.toml is not a safe regular file"
  [[ "$(stat -c '%u' -- /etc/helix/helix.toml)" == "0" ]] ||
    fail "/etc/helix/helix.toml must be owned by root"
  printf 'Preserving existing configuration: /etc/helix/helix.toml\n'
else
  config_staging="$(mktemp --tmpdir=/etc/helix '.helix.toml.helix-new.XXXXXXXX')"
  install -o root -g helix -m 0640 -- "$bundle_dir/packaging/helix.toml" "$config_staging"
  verify_payload_file ./packaging/helix.toml "$config_staging"
  mv -T -- "$config_staging" /etc/helix/helix.toml
  config_staging=""
fi

# A valid configured state database (including one at an administrator-selected
# data_dir) is an existing installation. The fixed-path existence bit was
# captured before tmpfiles or any CLI invocation so a normal upgrade never
# generates a replacement setup token.
configured_state_readable_before=0
if runuser -u helix -- \
  env -i PATH=/usr/bin:/bin \
  "${HELIX_STAGED_PATHS[/usr/bin/helixctl]}" \
  --config /etc/helix/helix.toml \
  status >/dev/null 2>&1; then
  configured_state_readable_before=1
fi
bootstrap_required=0
if ((critical_state_existed_before == 0 && configured_state_readable_before == 0)); then
  bootstrap_required=1
fi

helix_capture_service_state
if ((bootstrap_required == 1 && HELIX_PRIOR_ACTIVE == 1)); then
  fail "refusing fresh setup initialization while an existing helixd service is active without readable state"
fi
helix_create_package_snapshot "$HELIX_PRIOR_ACTIVE" "$HELIX_PRIOR_ENABLED"
transaction_snapshot_id=${HELIX_CREATED_SNAPSHOT##*/}

rollback_armed=1
helix_stop_service

for target in "${HELIX_MANAGED_TARGETS[@]}"; do
  helix_replace_managed_target "$target"
done

for target in "${!managed_payload_keys[@]}"; do
  helix_validate_existing_managed_target "$target"
  verify_payload_file "${managed_payload_keys[$target]}" "$target"
done
helix_validate_existing_managed_target /usr/share/helix/web
verify_payload_tree ./web /usr/share/helix/web

systemctl daemon-reload
/usr/bin/helixd --version
/usr/bin/helixctl --version

if ((bootstrap_required == 1)); then
  printf 'Initializing fresh owner setup state; the following token is shown once.\n' >&2
  bootstrap_attempted=1
  runuser -u helix -- \
    env -i PATH=/usr/bin:/bin \
    /usr/bin/helixctl \
    --config /etc/helix/helix.toml \
    setup-token
fi

if ((start_service == 1)); then
  systemctl enable --now helixd.service
  systemctl is-active --quiet helixd.service || fail "helixd did not remain active"
  runuser -u helix -- \
    env -i PATH=/usr/bin:/bin \
    /usr/bin/helixctl \
    --config /etc/helix/helix.toml \
    ready --timeout-seconds 20 || fail "helixd did not pass bounded readiness checks"
  printf 'Helix is active and passed liveness, critical-state API, and compiled UI readiness checks.\n'
elif ((HELIX_PRIOR_ACTIVE == 1)); then
  printf 'Helix was active before installation and remains stopped because --no-start was requested.\n'
else
  printf 'Helix was installed without starting the service.\n'
fi

# Retired sibling content is deleted only after the replacement and requested
# service state have validated. A cleanup failure is still transaction-fatal.
helix_cleanup_transients
cleanup_config_staging
rollback_armed=0
trap - EXIT HUP INT TERM

printf 'Package-file rollback snapshot: %s\n' "$transaction_snapshot_id"
printf 'Rollback command: sudo %s/rollback-local.sh --snapshot %s\n' \
  "$bundle_dir" "$transaction_snapshot_id"
printf 'Local lifecycle status: SCOPED-LIFECYCLE-TESTED (public support blocked; broader Linux fault-injection testing required).\n'
