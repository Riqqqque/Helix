#!/usr/bin/env bash
set -Eeuo pipefail

umask 077

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
common_file="$script_dir/package-common.sh"
snapshot_id=""

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
Usage: sudo ./rollback-local.sh --snapshot SNAPSHOT_ID

Restore only the package-owned Helix executables, static web assets, systemd
unit, sysusers file, and tmpfiles file recorded in an exact rollback snapshot.
Configuration, service-account records, databases, instances, caches, backups,
and other user data are never restored or removed by this command.
USAGE
}

while (($# > 0)); do
  case "$1" in
    --snapshot)
      (($# >= 2)) || fail "--snapshot requires a snapshot ID"
      [[ -z "$snapshot_id" ]] || fail "--snapshot may be supplied only once"
      snapshot_id=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *) fail "unknown argument: $1" ;;
  esac
done
[[ -n "$snapshot_id" ]] || fail "--snapshot is required"
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
helix_prepare_package_parents

recovery_armed=0
safety_snapshot_id=""

finish_rollback() {
  local status=$?
  local restore_status=0
  local cleanup_status=0

  trap - EXIT HUP INT TERM
  set +e
  if ((recovery_armed == 1)); then
    printf 'Rollback failed after helixd was stopped; restoring safety snapshot %s.\n' \
      "$safety_snapshot_id" >&2
    (
      set -Eeuo pipefail
      helix_restore_package_snapshot "$safety_snapshot_id"
    )
    restore_status=$?
    if ((restore_status == 0)); then
      printf 'The package files and service state from before this rollback were restored.\n' >&2
    else
      printf 'error: safety restoration failed; snapshot retained: %s/%s\n' \
        "$HELIX_ROLLBACK_ROOT" "$safety_snapshot_id" >&2
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

trap finish_rollback EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# Full snapshot validation and sibling staging complete before the service is
# stopped. This also captures the target snapshot's recorded service state.
helix_stage_snapshot "$snapshot_id"
desired_snapshot=$HELIX_VALIDATED_SNAPSHOT
desired_active=$HELIX_SNAPSHOT_PRIOR_ACTIVE
desired_enabled=$HELIX_SNAPSHOT_PRIOR_ENABLED

helix_capture_service_state
helix_create_package_snapshot "$HELIX_PRIOR_ACTIVE" "$HELIX_PRIOR_ENABLED"
safety_snapshot_id=${HELIX_CREATED_SNAPSHOT##*/}

recovery_armed=1
helix_stop_service
for target in "${HELIX_MANAGED_TARGETS[@]}"; do
  if [[ "${HELIX_SNAPSHOT_PRESENCE[$target]}" == "present" ]]; then
    helix_replace_managed_target "$target"
  else
    helix_remove_managed_target "$target"
  fi
done
helix_verify_snapshot_restored "$desired_snapshot"
helix_restore_service_state "$desired_active" "$desired_enabled"
helix_cleanup_transients

recovery_armed=0
trap - EXIT HUP INT TERM

printf 'Restored package snapshot: %s\n' "$snapshot_id"
printf 'Safety snapshot retained: %s\n' "$safety_snapshot_id"
printf 'Configuration and user data were not changed.\n'
printf 'Local lifecycle status: SCOPED-LIFECYCLE-TESTED (public support blocked; broader Linux fault-injection testing required).\n'
