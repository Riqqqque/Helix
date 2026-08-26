#!/usr/bin/env bash
set -Eeuo pipefail

umask 077

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
common_file="$script_dir/package-common.sh"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: sudo ./uninstall-local.sh

Remove only Helix package-owned executables, static web assets, systemd unit,
sysusers file, and tmpfiles file. This conservative uninstall preserves
/etc/helix, the helix account, /var/lib/helix, /var/cache/helix, instances,
backups, and all package rollback snapshots.
USAGE
}

while (($# > 0)); do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done
[[ -f "$common_file" && ! -L "$common_file" ]] || fail "package-common.sh is missing or unsafe"

# shellcheck source=scripts/package-common.sh
source "$common_file"

helix_require_commands \
  chmod chown cmp cp cut date find flock id install mktemp mountpoint mv realpath rm rmdir \
  sha256sum sort stat systemctl uname xargs
helix_require_linux_root
helix_acquire_package_lock
helix_assert_system_paths_safe
helix_prepare_metadata_roots

recovery_armed=0
uninstall_snapshot_id=""

finish_uninstall() {
  local status=$?
  local restore_status=0
  local cleanup_status=0

  trap - EXIT HUP INT TERM
  set +e
  if ((recovery_armed == 1)); then
    printf 'Uninstall failed after helixd was stopped; restoring package snapshot %s.\n' \
      "$uninstall_snapshot_id" >&2
    (
      set -Eeuo pipefail
      helix_restore_package_snapshot "$uninstall_snapshot_id"
    )
    restore_status=$?
    if ((restore_status == 0)); then
      printf 'Previous package files and service state were restored.\n' >&2
    else
      printf 'error: automatic package restoration failed; snapshot retained: %s/%s\n' \
        "$HELIX_ROLLBACK_ROOT" "$uninstall_snapshot_id" >&2
      status=1
    fi
  fi
  (
    set -Eeuo pipefail
    helix_cleanup_transients
  )
  cleanup_status=$?
  if ((cleanup_status != 0)); then
    printf 'warning: one or more validated package staging paths could not be cleaned\n' >&2
    status=1
  fi
  exit "$status"
}

trap finish_uninstall EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

helix_capture_service_state
helix_create_package_snapshot "$HELIX_PRIOR_ACTIVE" "$HELIX_PRIOR_ENABLED"
uninstall_snapshot_id=${HELIX_CREATED_SNAPSHOT##*/}

recovery_armed=1
helix_stop_service
if [[ -f /usr/lib/systemd/system/helixd.service ]]; then
  systemctl disable helixd.service >/dev/null
fi

for target in "${HELIX_MANAGED_TARGETS[@]}"; do
  helix_remove_managed_target "$target"
done
systemctl daemon-reload

for target in "${HELIX_MANAGED_TARGETS[@]}"; do
  [[ ! -e "$target" && ! -L "$target" ]] ||
    fail "uninstall did not remove package target: $target"
done

helix_cleanup_transients
recovery_armed=0
trap - EXIT HUP INT TERM

# Remove the package parent only when it became empty. A pre-existing directory
# with administrator-managed content is deliberately preserved.
if [[ -d /usr/share/helix && ! -L /usr/share/helix ]]; then
  rmdir -- /usr/share/helix 2>/dev/null || true
fi

printf 'Removed Helix package files.\n'
printf 'Preserved configuration, service account, state, cache, instances, backups, and rollback material.\n'
printf 'Recovery snapshot: %s\n' "$uninstall_snapshot_id"
printf 'Rollback command: sudo %s/rollback-local.sh --snapshot %s\n' \
  "$script_dir" "$uninstall_snapshot_id"
printf 'Local lifecycle status: SCOPED-LIFECYCLE-TESTED (public support blocked; broader Linux fault-injection testing required).\n'
