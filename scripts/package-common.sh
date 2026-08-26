#!/usr/bin/env bash
# Shared package-file transaction helpers. This file is sourced by the local
# install, rollback, and uninstall entry points; it is not an installer itself.

if ((BASH_VERSINFO[0] < 4)); then
  printf 'error: Helix package tools require Bash 4 or newer\n' >&2
  exit 1
fi

readonly HELIX_PACKAGE_LOCK_FILE=/run/lock/helix-package.lock
readonly HELIX_PACKAGE_METADATA_ROOT=/var/lib/helix-package
readonly HELIX_ROLLBACK_ROOT=/var/lib/helix-package/rollbacks

readonly -a HELIX_MANAGED_TARGETS=(
  /usr/bin/helixd
  /usr/bin/helixctl
  /usr/share/helix/web
  /usr/lib/systemd/system/helixd.service
  /usr/lib/sysusers.d/helix.conf
  /usr/lib/tmpfiles.d/helix.conf
)

declare -Ag HELIX_TARGET_KIND=(
  [/usr/bin/helixd]=file
  [/usr/bin/helixctl]=file
  [/usr/share/helix/web]=tree
  [/usr/lib/systemd/system/helixd.service]=file
  [/usr/lib/sysusers.d/helix.conf]=file
  [/usr/lib/tmpfiles.d/helix.conf]=file
)

declare -Ag HELIX_TARGET_MODE=(
  [/usr/bin/helixd]=755
  [/usr/bin/helixctl]=755
  [/usr/lib/systemd/system/helixd.service]=644
  [/usr/lib/sysusers.d/helix.conf]=644
  [/usr/lib/tmpfiles.d/helix.conf]=644
)

declare -Ag HELIX_STAGED_PATHS=()
declare -Ag HELIX_SNAPSHOT_PRESENCE=()
declare -ag HELIX_RETIRED_PATHS=()
HELIX_PENDING_SNAPSHOT=""
HELIX_CREATED_SNAPSHOT=""
HELIX_VALIDATED_SNAPSHOT=""
HELIX_SNAPSHOT_PRIOR_ACTIVE=""
HELIX_SNAPSHOT_PRIOR_ENABLED=""
HELIX_PACKAGE_LOCK_FD=""

helix_package_fail() {
  printf 'error: %s\n' "$*" >&2
  return 1
}

helix_require_linux_root() {
  [[ "$(uname -s)" == "Linux" ]] ||
    helix_package_fail "the Helix package lifecycle supports Linux/systemd hosts only"
  [[ "$(id -u)" -eq 0 ]] ||
    helix_package_fail "run this package operation as root, for example with sudo"
  [[ -d /run/systemd/system ]] ||
    helix_package_fail "systemd is not the active service manager"
}

helix_require_commands() {
  local command_name
  for command_name in "$@"; do
    command -v "$command_name" >/dev/null 2>&1 ||
      helix_package_fail "required package command is unavailable: $command_name"
  done
}

helix_assert_root_owned() {
  local path=$1
  local ownership
  ownership="$(stat -c '%u:%g' -- "$path")" || return 1
  [[ "$ownership" == "0:0" ]] ||
    helix_package_fail "package path is not owned by root:root: $path"
}

helix_assert_not_service_writable() {
  local path=$1
  local writable_entry

  writable_entry="$(find "$path" -xdev -perm /022 -print -quit)"
  [[ -z "$writable_entry" ]] ||
    helix_package_fail "package content is writable by its service group or other users: $writable_entry"
}

helix_assert_secure_package_directory() {
  local path=$1
  local writable_entry

  [[ -d "$path" && ! -L "$path" ]] ||
    helix_package_fail "package parent is not a safe directory: $path"
  helix_assert_root_owned "$path"
  writable_entry="$(find "$path" -xdev -maxdepth 0 -perm /022 -print -quit)"
  [[ -z "$writable_entry" ]] ||
    helix_package_fail "package parent is writable by a non-root account: $path"
}

helix_ensure_root_directory() {
  local path=$1
  local mode=$2
  if [[ -e "$path" || -L "$path" ]]; then
    [[ -d "$path" && ! -L "$path" ]] ||
      helix_package_fail "required package directory is unsafe: $path"
    helix_assert_root_owned "$path"
  else
    install -d -o root -g root -m "$mode" -- "$path"
  fi
}

helix_acquire_package_lock() {
  helix_ensure_root_directory /run/lock 0755

  if [[ ! -e "$HELIX_PACKAGE_LOCK_FILE" && ! -L "$HELIX_PACKAGE_LOCK_FILE" ]]; then
    (
      umask 077
      set -o noclobber
      : > "$HELIX_PACKAGE_LOCK_FILE"
    ) 2>/dev/null || true
  fi

  [[ -f "$HELIX_PACKAGE_LOCK_FILE" && ! -L "$HELIX_PACKAGE_LOCK_FILE" ]] ||
    helix_package_fail "package lock is not a safe regular file"
  helix_assert_root_owned "$HELIX_PACKAGE_LOCK_FILE"
  chmod 0600 -- "$HELIX_PACKAGE_LOCK_FILE"

  exec {HELIX_PACKAGE_LOCK_FD}<>"$HELIX_PACKAGE_LOCK_FILE"
  flock --exclusive --nonblock "$HELIX_PACKAGE_LOCK_FD" ||
    helix_package_fail "another Helix package operation is already running"
}

helix_prepare_metadata_roots() {
  for path in "$HELIX_PACKAGE_METADATA_ROOT" "$HELIX_ROLLBACK_ROOT"; do
    helix_ensure_root_directory "$path" 0700
    chmod 0700 -- "$path"
  done
}

helix_assert_system_paths_safe() {
  local path
  for path in \
    /usr/bin \
    /usr/share \
    /usr/lib \
    /etc \
    /var/lib \
    /var/cache \
    /run; do
    [[ -d "$path" && ! -L "$path" ]] ||
      helix_package_fail "refusing to operate through an unsafe system root: $path"
  done

  for path in \
    /usr/share/helix \
    /etc/helix \
    /var/lib/helix \
    /var/lib/helix/state \
    /var/lib/helix/metrics \
    /var/cache/helix \
    /run/helix \
    "$HELIX_PACKAGE_METADATA_ROOT" \
    "$HELIX_ROLLBACK_ROOT" \
    /etc/helix/helix.toml \
    "${HELIX_MANAGED_TARGETS[@]}"; do
    [[ ! -L "$path" ]] ||
      helix_package_fail "refusing to operate through a symlinked Helix path: $path"
  done
}

helix_prepare_package_parents() {
  local path
  for path in \
    /usr/share/helix \
    /usr/lib/systemd \
    /usr/lib/systemd/system \
    /usr/lib/sysusers.d \
    /usr/lib/tmpfiles.d; do
    helix_ensure_root_directory "$path" 0755
    helix_assert_secure_package_directory "$path"
  done
}

helix_validate_package_tree() {
  local tree=$1
  local unsafe_entry
  local non_root_entry
  local tree_entry
  local relative_entry

  [[ -d "$tree" && ! -L "$tree" ]] ||
    helix_package_fail "package asset tree is not a safe directory: $tree"
  ! mountpoint -q "$tree" ||
    helix_package_fail "package asset tree must not be a mount point: $tree"
  unsafe_entry="$(
    find "$tree" -xdev \
      \( -type l -o -type b -o -type c -o -type p -o -type s \) \
      -print -quit
  )"
  [[ -z "$unsafe_entry" ]] ||
    helix_package_fail "package asset tree contains an unsafe entry: $unsafe_entry"
  non_root_entry="$(
    find "$tree" -xdev \( ! -uid 0 -o ! -gid 0 \) -print -quit
  )"
  [[ -z "$non_root_entry" ]] ||
    helix_package_fail "package asset tree contains a non-root-owned entry: $non_root_entry"
  helix_assert_not_service_writable "$tree"
  while IFS= read -r -d '' tree_entry; do
    relative_entry="./${tree_entry#"$tree"/}"
    [[ "$relative_entry" =~ ^\./[A-Za-z0-9._/+@-]+$ ]] ||
      helix_package_fail "package asset tree contains an unsupported path: $relative_entry"
    if [[ -d "$tree_entry" ]]; then
      ! mountpoint -q "$tree_entry" ||
        helix_package_fail "package asset tree contains a mount point: $relative_entry"
      [[ "$(stat -c '%a' -- "$tree_entry")" == "755" ]] ||
        helix_package_fail "package asset directory must have mode 0755: $relative_entry"
    else
      [[ "$(stat -c '%a' -- "$tree_entry")" == "644" ]] ||
        helix_package_fail "package asset file must have mode 0644: $relative_entry"
    fi
  done < <(find "$tree" -xdev -mindepth 1 -print0)
  [[ "$(stat -c '%a' -- "$tree")" == "755" ]] ||
    helix_package_fail "package asset root must have mode 0755: $tree"
}

helix_validate_existing_managed_target() {
  local target=$1
  local kind=${HELIX_TARGET_KIND[$target]:-}

  [[ -n "$kind" ]] || helix_package_fail "path is outside the package allowlist: $target"
  if [[ ! -e "$target" && ! -L "$target" ]]; then
    return 0
  fi
  [[ ! -L "$target" ]] || helix_package_fail "managed package target is a symlink: $target"
  ! mountpoint -q "$target" ||
    helix_package_fail "managed package target must not be a mount point: $target"

  if [[ "$kind" == "tree" ]]; then
    helix_validate_package_tree "$target"
  else
    [[ -f "$target" ]] ||
      helix_package_fail "managed package target is not a regular file: $target"
    helix_assert_root_owned "$target"
    helix_assert_not_service_writable "$target"
    [[ "$(stat -c '%a' -- "$target")" == "${HELIX_TARGET_MODE[$target]}" ]] ||
      helix_package_fail "managed package target has an unexpected mode: $target"
  fi
}

helix_capture_service_state() {
  # These globals are consumed by the lifecycle script that sources this file.
  # shellcheck disable=SC2034
  HELIX_PRIOR_ACTIVE=0
  # shellcheck disable=SC2034
  HELIX_PRIOR_ENABLED=0
  if systemctl is-active --quiet helixd.service >/dev/null 2>&1; then
    # shellcheck disable=SC2034
    HELIX_PRIOR_ACTIVE=1
  fi
  if systemctl is-enabled --quiet helixd.service >/dev/null 2>&1; then
    # shellcheck disable=SC2034
    HELIX_PRIOR_ENABLED=1
  fi
}

helix_create_package_snapshot() {
  local prior_active=$1
  local prior_enabled=$2
  local pending_suffix
  local snapshot_id
  local final_path
  local target
  local destination
  local presence

  [[ "$prior_active" =~ ^[01]$ && "$prior_enabled" =~ ^[01]$ ]] ||
    helix_package_fail "invalid service state supplied for package snapshot"
  helix_prepare_metadata_roots

  HELIX_PENDING_SNAPSHOT="$(mktemp -d "$HELIX_ROLLBACK_ROOT/.pending.XXXXXXXX")"
  chown root:root -- "$HELIX_PENDING_SNAPSHOT"
  chmod 0700 -- "$HELIX_PENDING_SNAPSHOT"
  install -d -o root -g root -m 0700 -- "$HELIX_PENDING_SNAPSHOT/files"

  pending_suffix=${HELIX_PENDING_SNAPSHOT##*.pending.}
  snapshot_id="snapshot-$(date -u +%Y%m%dT%H%M%SZ)-$pending_suffix"
  final_path="$HELIX_ROLLBACK_ROOT/$snapshot_id"
  [[ ! -e "$final_path" && ! -L "$final_path" ]] ||
    helix_package_fail "generated rollback snapshot already exists"

  {
    printf 'HELIX_PACKAGE_SNAPSHOT_V1\n'
    printf 'snapshot_id %s\n' "$snapshot_id"
    printf 'prior_active %s\n' "$prior_active"
    printf 'prior_enabled %s\n' "$prior_enabled"
    for target in "${HELIX_MANAGED_TARGETS[@]}"; do
      helix_validate_existing_managed_target "$target"
      if [[ -e "$target" ]]; then
        presence=present
        destination="$HELIX_PENDING_SNAPSHOT/files$target"
        install -d -o root -g root -m 0700 -- "${destination%/*}"
        cp -a -- "$target" "$destination"
      else
        presence=absent
      fi
      printf 'target %s %s\n' "$presence" "$target"
    done
  } > "$HELIX_PENDING_SNAPSHOT/MANIFEST"
  chown root:root -- "$HELIX_PENDING_SNAPSHOT/MANIFEST"
  chmod 0600 -- "$HELIX_PENDING_SNAPSHOT/MANIFEST"

  (
    cd -- "$HELIX_PENDING_SNAPSHOT" || exit 1
    find ./MANIFEST ./files -type f -print0 |
      LC_ALL=C sort -z |
      xargs -0 sha256sum > SHA256SUMS
  )
  chown root:root -- "$HELIX_PENDING_SNAPSHOT/SHA256SUMS"
  chmod 0600 -- "$HELIX_PENDING_SNAPSHOT/SHA256SUMS"

  mv -T -- "$HELIX_PENDING_SNAPSHOT" "$final_path"
  HELIX_PENDING_SNAPSHOT=""
  # shellcheck disable=SC2034
  HELIX_CREATED_SNAPSHOT="$final_path"
}

helix_resolve_snapshot_id() {
  local snapshot_id=$1
  local requested_path
  local resolved_path

  [[ "$snapshot_id" =~ ^snapshot-[0-9]{8}T[0-9]{6}Z-[A-Za-z0-9]{8}$ ]] ||
    helix_package_fail "rollback snapshot ID has an invalid format"
  requested_path="$HELIX_ROLLBACK_ROOT/$snapshot_id"
  [[ -d "$requested_path" && ! -L "$requested_path" ]] ||
    helix_package_fail "rollback snapshot does not exist: $snapshot_id"
  resolved_path="$(realpath -e -- "$requested_path")"
  [[ "$resolved_path" == "$requested_path" ]] ||
    helix_package_fail "rollback snapshot resolves outside its exact path"
  HELIX_VALIDATED_SNAPSHOT="$resolved_path"
}

helix_validate_snapshot_checksums() {
  local snapshot=$1
  local manifest_line
  local manifest_path
  local manifest_file
  local resolved_file
  local snapshot_file
  local relative_file
  local entries=0
  local -A checksum_paths=()

  [[ -f "$snapshot/SHA256SUMS" && ! -L "$snapshot/SHA256SUMS" ]] ||
    helix_package_fail "rollback checksum manifest is missing or unsafe"
  while IFS= read -r manifest_line; do
    [[ "$manifest_line" =~ ^[0-9a-f]{64}\ \ \./[A-Za-z0-9._/+@-]+$ ]] ||
      helix_package_fail "rollback checksum manifest contains an unsafe entry"
    manifest_path=${manifest_line:66}
    [[ -z "${checksum_paths[$manifest_path]+present}" ]] ||
      helix_package_fail "rollback checksum manifest contains a duplicate path"
    case "/$manifest_path/" in
      *"/../"*|*"//"*) helix_package_fail "rollback checksum path escapes its snapshot" ;;
    esac
    manifest_file="$snapshot/${manifest_path#./}"
    [[ -f "$manifest_file" && ! -L "$manifest_file" ]] ||
      helix_package_fail "rollback checksum references a missing or unsafe file"
    resolved_file="$(realpath -e -- "$manifest_file")"
    case "$resolved_file" in
      "$snapshot"/*) ;;
      *) helix_package_fail "rollback checksum resolves outside its snapshot" ;;
    esac
    checksum_paths["$manifest_path"]=1
    ((entries += 1))
  done < "$snapshot/SHA256SUMS"
  ((entries > 0)) || helix_package_fail "rollback checksum manifest is empty"

  while IFS= read -r -d '' snapshot_file; do
    relative_file="./${snapshot_file#"$snapshot"/}"
    [[ -n "${checksum_paths[$relative_file]+present}" ]] ||
      helix_package_fail "rollback snapshot contains an unchecksummed file: $relative_file"
  done < <(find "$snapshot" -xdev -type f ! -path "$snapshot/SHA256SUMS" -print0)

  (
    cd -- "$snapshot"
    sha256sum --check --strict SHA256SUMS
  ) >/dev/null || helix_package_fail "rollback snapshot checksum verification failed"
}

helix_validate_package_snapshot() {
  local snapshot_id=$1
  local snapshot
  local unsafe_entry
  local non_root_entry
  local -a manifest_lines=()
  local expected_lines
  local index
  local target
  local line
  local presence
  local snapshot_target
  local snapshot_file
  local snapshot_directory
  local snapshot_top_entry
  local snapshot_top_name
  local relative_target
  local relative_directory

  helix_prepare_metadata_roots
  helix_resolve_snapshot_id "$snapshot_id"
  snapshot=$HELIX_VALIDATED_SNAPSHOT
  helix_assert_root_owned "$snapshot"
  ! mountpoint -q "$snapshot" ||
    helix_package_fail "rollback snapshot must not be a mount point"
  [[ "$(stat -c '%a' -- "$snapshot")" == "700" ]] ||
    helix_package_fail "rollback snapshot root must have mode 0700"

  unsafe_entry="$(
    find "$snapshot" -xdev \
      \( -type l -o -type b -o -type c -o -type p -o -type s \) \
      -print -quit
  )"
  [[ -z "$unsafe_entry" ]] ||
    helix_package_fail "rollback snapshot contains an unsafe entry: $unsafe_entry"
  non_root_entry="$(
    find "$snapshot" -xdev \( ! -uid 0 -o ! -gid 0 \) -print -quit
  )"
  [[ -z "$non_root_entry" ]] ||
    helix_package_fail "rollback snapshot contains a non-root-owned entry: $non_root_entry"
  helix_assert_not_service_writable "$snapshot"

  [[ -f "$snapshot/MANIFEST" && ! -L "$snapshot/MANIFEST" ]] ||
    helix_package_fail "rollback snapshot manifest is missing or unsafe"
  [[ -f "$snapshot/SHA256SUMS" && ! -L "$snapshot/SHA256SUMS" ]] ||
    helix_package_fail "rollback checksum manifest is missing or unsafe"
  [[ "$(stat -c '%a' -- "$snapshot/MANIFEST")" == "600" ]] ||
    helix_package_fail "rollback snapshot manifest must have mode 0600"
  [[ "$(stat -c '%a' -- "$snapshot/SHA256SUMS")" == "600" ]] ||
    helix_package_fail "rollback checksum manifest must have mode 0600"
  [[ -d "$snapshot/files" && ! -L "$snapshot/files" ]] ||
    helix_package_fail "rollback snapshot files directory is missing or unsafe"
  [[ "$(stat -c '%a' -- "$snapshot/files")" == "700" ]] ||
    helix_package_fail "rollback snapshot files directory must have mode 0700"
  while IFS= read -r -d '' snapshot_top_entry; do
    snapshot_top_name=${snapshot_top_entry##*/}
    case "$snapshot_top_name" in
      MANIFEST|SHA256SUMS|files) ;;
      *) helix_package_fail "rollback snapshot contains an unexpected top-level entry" ;;
    esac
  done < <(find "$snapshot" -xdev -mindepth 1 -maxdepth 1 -print0)
  mapfile -t manifest_lines < "$snapshot/MANIFEST"
  expected_lines=$((4 + ${#HELIX_MANAGED_TARGETS[@]}))
  ((${#manifest_lines[@]} == expected_lines)) ||
    helix_package_fail "rollback snapshot manifest has an unexpected shape"
  [[ "${manifest_lines[0]}" == "HELIX_PACKAGE_SNAPSHOT_V1" ]] ||
    helix_package_fail "rollback snapshot manifest version is unsupported"
  [[ "${manifest_lines[1]}" == "snapshot_id $snapshot_id" ]] ||
    helix_package_fail "rollback snapshot ID does not match its directory"
  [[ "${manifest_lines[2]}" =~ ^prior_active\ ([01])$ ]] ||
    helix_package_fail "rollback snapshot active state is invalid"
  HELIX_SNAPSHOT_PRIOR_ACTIVE=${BASH_REMATCH[1]}
  [[ "${manifest_lines[3]}" =~ ^prior_enabled\ ([01])$ ]] ||
    helix_package_fail "rollback snapshot enabled state is invalid"
  HELIX_SNAPSHOT_PRIOR_ENABLED=${BASH_REMATCH[1]}

  HELIX_SNAPSHOT_PRESENCE=()
  index=4
  for target in "${HELIX_MANAGED_TARGETS[@]}"; do
    line=${manifest_lines[$index]}
    case "$line" in
      "target present $target") presence=present ;;
      "target absent $target") presence=absent ;;
      *) helix_package_fail "rollback snapshot target list is not exact" ;;
    esac
    HELIX_SNAPSHOT_PRESENCE["$target"]=$presence
    snapshot_target="$snapshot/files$target"
    if [[ "$presence" == "present" ]]; then
      if [[ "${HELIX_TARGET_KIND[$target]}" == "tree" ]]; then
        [[ -d "$snapshot_target" && ! -L "$snapshot_target" ]] ||
          helix_package_fail "rollback snapshot asset tree is missing"
        [[ -f "$snapshot_target/index.html" && ! -L "$snapshot_target/index.html" ]] ||
          helix_package_fail "rollback snapshot asset tree has no safe index.html"
        helix_validate_package_tree "$snapshot_target"
      else
        [[ -f "$snapshot_target" && ! -L "$snapshot_target" ]] ||
          helix_package_fail "rollback snapshot package file is missing: $target"
        [[ "$(stat -c '%a' -- "$snapshot_target")" == "${HELIX_TARGET_MODE[$target]}" ]] ||
          helix_package_fail "rollback snapshot package file has an unexpected mode: $target"
      fi
    elif [[ -e "$snapshot_target" || -L "$snapshot_target" ]]; then
      helix_package_fail "rollback snapshot stores a target marked absent: $target"
    fi
    ((index += 1))
  done

  if [[ -d "$snapshot/files" ]]; then
    while IFS= read -r -d '' snapshot_file; do
      relative_target="/${snapshot_file#"$snapshot/files"/}"
      case "$relative_target" in
        /usr/bin/helixd|/usr/bin/helixctl|/usr/lib/systemd/system/helixd.service|/usr/lib/sysusers.d/helix.conf|/usr/lib/tmpfiles.d/helix.conf) ;;
        /usr/share/helix/web/*)
          [[ "${HELIX_SNAPSHOT_PRESENCE[/usr/share/helix/web]}" == "present" ]] ||
            helix_package_fail "rollback snapshot has unexpected web assets"
          ;;
        *) helix_package_fail "rollback snapshot contains a file outside the package allowlist" ;;
      esac
    done < <(find "$snapshot/files" -xdev -type f -print0)

    while IFS= read -r -d '' snapshot_directory; do
      ! mountpoint -q "$snapshot_directory" ||
        helix_package_fail "rollback snapshot contains a mount point"
      if [[ "$snapshot_directory" == "$snapshot/files" ]]; then
        relative_directory=/
      else
        relative_directory="/${snapshot_directory#"$snapshot/files"/}"
      fi
      case "$relative_directory" in
        /|/usr|/usr/bin|/usr/share|/usr/share/helix|/usr/lib|/usr/lib/systemd|/usr/lib/systemd/system|/usr/lib/sysusers.d|/usr/lib/tmpfiles.d) ;;
        /usr/share/helix/web|/usr/share/helix/web/*)
          [[ "${HELIX_SNAPSHOT_PRESENCE[/usr/share/helix/web]}" == "present" ]] ||
            helix_package_fail "rollback snapshot has unexpected web directories"
          ;;
        *) helix_package_fail "rollback snapshot contains a directory outside the package allowlist" ;;
      esac
    done < <(find "$snapshot/files" -xdev -type d -print0)
  fi

  helix_validate_snapshot_checksums "$snapshot"
}

helix_tree_checksums() {
  local tree=$1
  (
    cd -- "$tree" || exit 1
    find . -xdev -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum
  )
}

helix_stage_file() {
  local source=$1
  local target=$2
  local mode=$3
  local parent=${target%/*}
  local base=${target##*/}
  local staged

  [[ -f "$source" && ! -L "$source" ]] ||
    helix_package_fail "staged source is not a safe file: $source"
  [[ "${HELIX_TARGET_KIND[$target]:-}" == "file" ]] ||
    helix_package_fail "staged file target is outside the package allowlist: $target"
  helix_ensure_root_directory "$parent" 0755
  helix_assert_secure_package_directory "$parent"
  staged="$(mktemp --tmpdir="$parent" ".$base.helix-new.XXXXXXXX")"
  install -o root -g root -m "$mode" -- "$source" "$staged"
  [[ "$(sha256sum -- "$source" | cut -d ' ' -f 1)" == "$(sha256sum -- "$staged" | cut -d ' ' -f 1)" ]] ||
    helix_package_fail "staged package file did not verify: $target"
  HELIX_STAGED_PATHS["$target"]=$staged
}

helix_stage_tree() {
  local source=$1
  local target=$2
  local parent=${target%/*}
  local staged

  [[ "$target" == "/usr/share/helix/web" ]] ||
    helix_package_fail "staged tree target is outside the package allowlist"
  [[ -f "$source/index.html" && ! -L "$source/index.html" ]] ||
    helix_package_fail "staged web assets do not contain a safe index.html"
  helix_ensure_root_directory "$parent" 0755
  helix_assert_secure_package_directory "$parent"
  staged="$(mktemp -d --tmpdir="$parent" '.helix-web-new.XXXXXXXX')"
  cp -a -- "$source/." "$staged/"
  chown -hR root:root -- "$staged"
  find "$staged" -type d -exec chmod 0755 {} +
  find "$staged" -type f -exec chmod 0644 {} +
  helix_validate_package_tree "$staged"
  cmp -s <(helix_tree_checksums "$source") <(helix_tree_checksums "$staged") ||
    helix_package_fail "staged web assets did not verify"
  HELIX_STAGED_PATHS["$target"]=$staged
}

helix_stage_snapshot() {
  local snapshot_id=$1
  local snapshot
  local target
  local source
  local mode

  helix_validate_package_snapshot "$snapshot_id"
  snapshot=$HELIX_VALIDATED_SNAPSHOT
  for target in "${HELIX_MANAGED_TARGETS[@]}"; do
    if [[ "${HELIX_SNAPSHOT_PRESENCE[$target]}" != "present" ]]; then
      continue
    fi
    source="$snapshot/files$target"
    if [[ "${HELIX_TARGET_KIND[$target]}" == "tree" ]]; then
      helix_stage_tree "$source" "$target"
    else
      mode=${HELIX_TARGET_MODE[$target]}
      helix_stage_file "$source" "$target" "$mode"
    fi
  done
}

helix_remove_transient_path() {
  local path=$1
  case "$path" in
    /usr/bin/.helixd.helix-new.*|/usr/bin/.helixctl.helix-new.*|/usr/lib/systemd/system/.helixd.service.helix-new.*|/usr/lib/sysusers.d/.helix.conf.helix-new.*|/usr/lib/tmpfiles.d/.helix.conf.helix-new.*)
      if [[ -e "$path" || -L "$path" ]]; then
        [[ -f "$path" && ! -L "$path" ]] || return 1
        rm -f -- "$path"
      fi
      ;;
    /usr/share/helix/.helix-web-new.*|/usr/share/helix/.helix-web-retired.*)
      if [[ -e "$path" || -L "$path" ]]; then
        [[ -d "$path" && ! -L "$path" ]] || return 1
        helix_validate_package_tree "$path" || return 1
        find "$path" -xdev -depth -delete
      fi
      ;;
    *) return 1 ;;
  esac
}

helix_cleanup_transients() {
  local target
  local path
  local cleanup_status=0

  for target in "${!HELIX_STAGED_PATHS[@]}"; do
    path=${HELIX_STAGED_PATHS[$target]}
    [[ -z "$path" ]] || helix_remove_transient_path "$path" || cleanup_status=1
  done
  HELIX_STAGED_PATHS=()
  for path in "${HELIX_RETIRED_PATHS[@]}"; do
    helix_remove_transient_path "$path" || cleanup_status=1
  done
  HELIX_RETIRED_PATHS=()

  if [[ -n "$HELIX_PENDING_SNAPSHOT" ]]; then
    case "$HELIX_PENDING_SNAPSHOT" in
      "$HELIX_ROLLBACK_ROOT"/.pending.*)
        if [[ -d "$HELIX_PENDING_SNAPSHOT" && ! -L "$HELIX_PENDING_SNAPSHOT" ]]; then
          helix_assert_root_owned "$HELIX_PENDING_SNAPSHOT" || cleanup_status=1
          find "$HELIX_PENDING_SNAPSHOT" -xdev -depth -delete || cleanup_status=1
        else
          cleanup_status=1
        fi
        ;;
      *) cleanup_status=1 ;;
    esac
    HELIX_PENDING_SNAPSHOT=""
  fi
  return "$cleanup_status"
}

helix_replace_managed_target() {
  local target=$1
  local staged=${HELIX_STAGED_PATHS[$target]:-}
  local parent
  local retired

  [[ -n "$staged" ]] || helix_package_fail "no staged package content for: $target"
  helix_validate_existing_managed_target "$target"
  if [[ "${HELIX_TARGET_KIND[$target]}" == "tree" ]]; then
    parent=${target%/*}
    if [[ -e "$target" ]]; then
      retired="$(mktemp -d --tmpdir="$parent" '.helix-web-retired.XXXXXXXX')"
      rmdir -- "$retired"
      mv -T -- "$target" "$retired"
      HELIX_RETIRED_PATHS+=("$retired")
    fi
    if ! mv -T -- "$staged" "$target"; then
      if [[ -n "${retired:-}" && ! -e "$target" && -d "$retired" ]]; then
        mv -T -- "$retired" "$target" || true
      fi
      return 1
    fi
  else
    mv -fT -- "$staged" "$target"
  fi
  unset 'HELIX_STAGED_PATHS[$target]'
}

helix_remove_managed_target() {
  local target=$1
  local kind=${HELIX_TARGET_KIND[$target]:-}

  [[ -n "$kind" ]] || helix_package_fail "refusing to remove a path outside the package allowlist"
  if [[ ! -e "$target" && ! -L "$target" ]]; then
    return 0
  fi
  helix_validate_existing_managed_target "$target"
  if [[ "$kind" == "tree" ]]; then
    find "$target" -xdev -depth -delete
  else
    rm -f -- "$target"
  fi
}

helix_stop_service() {
  local load_state

  load_state="$(systemctl show helixd.service --property=LoadState --value)" ||
    helix_package_fail "could not determine whether helixd is loaded"
  if [[ "$load_state" != "not-found" ]]; then
    systemctl stop helixd.service
  fi
  ! systemctl is-active --quiet helixd.service ||
    helix_package_fail "helixd remained active after the stop request"
}

helix_restore_service_state() {
  local active=$1
  local enabled=$2

  [[ "$active" =~ ^[01]$ && "$enabled" =~ ^[01]$ ]] ||
    helix_package_fail "cannot restore an invalid service state"
  systemctl daemon-reload
  if ((enabled == 1)); then
    [[ -f /usr/lib/systemd/system/helixd.service ]] ||
      helix_package_fail "cannot re-enable helixd because its unit is absent"
    systemctl enable helixd.service >/dev/null
    systemctl is-enabled --quiet helixd.service ||
      helix_package_fail "restored helixd did not remain enabled"
  else
    systemctl disable helixd.service >/dev/null 2>&1 || true
    ! systemctl is-enabled --quiet helixd.service ||
      helix_package_fail "restored helixd remained enabled"
  fi

  if ((active == 1)); then
    [[ -f /usr/lib/systemd/system/helixd.service ]] ||
      helix_package_fail "cannot restart helixd because its unit is absent"
    systemctl start helixd.service
    systemctl is-active --quiet helixd.service ||
      helix_package_fail "restored helixd did not remain active"
  else
    helix_stop_service
  fi
}

helix_verify_snapshot_restored() {
  local snapshot=$1
  local target
  local source

  for target in "${HELIX_MANAGED_TARGETS[@]}"; do
    source="$snapshot/files$target"
    if [[ "${HELIX_SNAPSHOT_PRESENCE[$target]}" == "absent" ]]; then
      [[ ! -e "$target" && ! -L "$target" ]] ||
        helix_package_fail "rollback did not remove package target marked absent: $target"
    elif [[ "${HELIX_TARGET_KIND[$target]}" == "tree" ]]; then
      helix_validate_package_tree "$target"
      cmp -s <(helix_tree_checksums "$source") <(helix_tree_checksums "$target") ||
        helix_package_fail "restored web assets do not match the rollback snapshot"
    else
      helix_validate_existing_managed_target "$target"
      [[ "$(sha256sum -- "$source" | cut -d ' ' -f 1)" == "$(sha256sum -- "$target" | cut -d ' ' -f 1)" ]] ||
        helix_package_fail "restored package file does not match its snapshot: $target"
    fi
  done
}

helix_restore_package_snapshot() {
  local snapshot_id=$1
  local snapshot
  local target

  helix_cleanup_transients
  helix_stage_snapshot "$snapshot_id"
  snapshot=$HELIX_VALIDATED_SNAPSHOT
  helix_stop_service

  for target in "${HELIX_MANAGED_TARGETS[@]}"; do
    if [[ "${HELIX_SNAPSHOT_PRESENCE[$target]}" == "present" ]]; then
      helix_replace_managed_target "$target"
    else
      helix_remove_managed_target "$target"
    fi
  done
  helix_verify_snapshot_restored "$snapshot"
  helix_restore_service_state "$HELIX_SNAPSHOT_PRIOR_ACTIVE" "$HELIX_SNAPSHOT_PRIOR_ENABLED"
  helix_cleanup_transients
}
