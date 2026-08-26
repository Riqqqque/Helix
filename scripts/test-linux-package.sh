#!/usr/bin/env bash
set -Eeuo pipefail

umask 077

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

[[ "${HELIX_EPHEMERAL_SYSTEM_TEST:-}" == "github-actions-clean-runner" ]] ||
  fail "this destructive package test is restricted to an explicitly confirmed clean ephemeral runner"
[[ "$(uname -s)" == "Linux" ]] || fail "the package lifecycle test requires Linux"
[[ "$(id -u)" -eq 0 ]] || fail "run the package lifecycle test as root"
[[ "$(ps -p 1 -o comm= | tr -d '[:space:]')" == "systemd" ]] ||
  fail "PID 1 is not systemd"

for command in \
  cp curl env find getent grep id install jq journalctl mktemp openssl ps readlink runuser \
  sed sha256sum sleep stat systemctl tail tr uname; do
  command -v "$command" >/dev/null 2>&1 || fail "required test command is unavailable: $command"
done

[[ $# -eq 1 ]] || fail "usage: test-linux-package.sh BUNDLE_DIRECTORY"
bundle_input=$1
[[ "$bundle_input" == /* ]] || fail "bundle path must be absolute"
[[ -d "$bundle_input" && ! -L "$bundle_input" ]] || fail "bundle is not a regular directory"
bundle_dir="$(readlink -e -- "$bundle_input")"
[[ -n "$bundle_dir" && "$bundle_dir" == "$bundle_input" ]] ||
  fail "bundle path must already be canonical"

for required in \
  SHA256SUMS \
  bin/helixd \
  bin/helixctl \
  web/index.html \
  install-local.sh \
  rollback-local.sh \
  uninstall-local.sh; do
  [[ -f "$bundle_dir/$required" && ! -L "$bundle_dir/$required" ]] ||
    fail "bundle input is missing a required regular file: $required"
done

# This test deliberately owns the fixed package paths. Refuse to run on a host
# with any sign of a real or previous Helix installation.
for path in \
  /usr/bin/helixd \
  /usr/bin/helixctl \
  /usr/share/helix \
  /usr/lib/systemd/system/helixd.service \
  /usr/lib/sysusers.d/helix.conf \
  /usr/lib/tmpfiles.d/helix.conf \
  /etc/helix \
  /var/lib/helix \
  /var/cache/helix \
  /run/helix \
  /run/lock/helix-package.lock \
  /etc/systemd/system/helixd.service \
  /etc/systemd/system/helixd.service.d \
  /run/systemd/system/helixd.service \
  /run/systemd/system/helixd.service.d \
  /usr/local/lib/systemd/system/helixd.service \
  /var/lib/helix-package; do
  [[ ! -e "$path" && ! -L "$path" ]] || fail "refusing to touch a non-clean host: $path exists"
done
if getent passwd helix >/dev/null || getent group helix >/dev/null; then
  fail "refusing to touch a host where the helix account or group already exists"
fi
if systemctl cat helixd.service >/dev/null 2>&1; then
  fail "refusing to touch a host where a helixd systemd unit is already visible"
fi

test_root="$(mktemp -d /tmp/helix-package-test.XXXXXXXX)"
[[ "$test_root" == /tmp/helix-package-test.* && -d "$test_root" && ! -L "$test_root" ]] ||
  fail "could not create a safe test root"
probe_root=""

cleanup() {
  if [[ -n "${probe_root:-}" && "$probe_root" == /run/helix-package-probe.* && -d "$probe_root" && ! -L "$probe_root" ]]; then
    find "$probe_root" -xdev -depth -delete
  fi
  if [[ -n "${test_root:-}" && "$test_root" == /tmp/helix-package-test.* && -d "$test_root" && ! -L "$test_root" ]]; then
    find "$test_root" -xdev -depth -delete
  fi
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

install_log="$test_root/install.log"
owner_response="$test_root/owner.json"
owner_payload="$test_root/owner-request.json"
cookie_jar="$test_root/cookies.txt"
health_response="$test_root/health.json"
overview_response="$test_root/overview.json"
csrf_response="$test_root/csrf.json"
login_payload="$test_root/login-request.json"
login_response="$test_root/login.json"

if ! "$bundle_dir/install-local.sh" --bundle "$bundle_dir" >"$install_log" 2>&1; then
  printf 'Redacted installer log follows:\n' >&2
  sed -E '/^[A-Za-z0-9_-]{43}$/c\[REDACTED SETUP TOKEN]' "$install_log" >&2
  printf 'Service journal follows:\n' >&2
  if ! journalctl --quiet --unit=helixd.service --no-pager --lines=100 --output=short-precise >&2; then
    printf 'warning: the Helix service journal could not be read\n' >&2
  fi
  fail "fresh installation failed"
fi

mapfile -t setup_tokens < <(grep -E '^[A-Za-z0-9_-]{43}$' "$install_log")
[[ "${#setup_tokens[@]}" -eq 1 ]] ||
  fail "fresh installation did not produce exactly one canonical setup token"
setup_token=${setup_tokens[0]}
unset setup_tokens
setup_token_canary=$setup_token

systemctl is-enabled --quiet helixd.service || fail "helixd was not enabled"
systemctl is-active --quiet helixd.service || fail "helixd was not active"
runuser -u helix -- env -i PATH=/usr/bin:/bin \
  /usr/bin/helixctl --config /etc/helix/helix.toml ready --timeout-seconds 20 >/dev/null

[[ "$(stat -c '%U:%G:%a' /usr/bin/helixd)" == "root:root:755" ]] || fail "helixd package mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /usr/bin/helixctl)" == "root:root:755" ]] || fail "helixctl package mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /var/lib/helix)" == "helix:helix:700" ]] || fail "state root mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /var/lib/helix/state)" == "helix:helix:700" ]] || fail "critical-state directory mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /var/lib/helix/metrics)" == "helix:helix:700" ]] || fail "metrics directory mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /var/cache/helix)" == "helix:helix:700" ]] || fail "cache directory mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /var/lib/helix/state/helix-state.db)" == "helix:helix:600" ]] ||
  fail "critical-state database mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /etc/helix)" == "root:helix:750" ]] ||
  fail "configuration directory mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /etc/helix/helix.toml)" == "root:helix:640" ]] ||
  fail "configuration file mode is incorrect"
[[ "$(stat -c '%U:%G:%a' /usr/share/helix/web/index.html)" == "root:root:644" ]] ||
  fail "compiled UI mode is incorrect"
[[ "$(systemctl show helixd.service --property=User --value)" == "helix" ]] ||
  fail "systemd did not run helixd as the dedicated account"
[[ "$(systemctl show helixd.service --property=Group --value)" == "helix" ]] ||
  fail "systemd did not apply the dedicated service group"
[[ "$(systemctl show helixd.service --property=UMask --value)" == "0077" ]] ||
  fail "systemd did not apply the private service umask"
[[ "$(systemctl show helixd.service --property=NoNewPrivileges --value)" == "yes" ]] ||
  fail "systemd did not apply NoNewPrivileges"
[[ "$(systemctl show helixd.service --property=ProtectSystem --value)" == "strict" ]] ||
  fail "systemd did not apply strict filesystem protection"
[[ "$(systemctl show helixd.service --property=LimitNOFILE --value)" == "1024" ]] ||
  fail "systemd did not apply the file-descriptor limit"

owner_password="$(openssl rand -hex 32)"
jq -n \
  --arg bootstrapToken "$setup_token" \
  --arg loginName "ci.owner" \
  --arg displayName "CI Owner" \
  --arg password "$owner_password" \
  '{bootstrapToken:$bootstrapToken,loginName:$loginName,displayName:$displayName,password:$password}' \
  >"$owner_payload"
unset setup_token

owner_status="$(
  curl --fail-with-body --silent --show-error \
    --cookie-jar "$cookie_jar" \
    --header 'Content-Type: application/json' \
    --header 'Origin: http://127.0.0.1:8080' \
    --header 'Sec-Fetch-Site: same-origin' \
    --data-binary "@$owner_payload" \
    --output "$owner_response" \
    --write-out '%{http_code}' \
    http://127.0.0.1:8080/api/v1/setup/owner
)"
[[ "$owner_status" == "201" ]] || fail "owner creation did not return HTTP 201"

csrf_token="$(jq -er '.csrfToken' "$owner_response")"
[[ "$csrf_token" =~ ^[A-Za-z0-9_-]{43}$ ]] || fail "owner response did not contain a canonical CSRF proof"

health_status="$(
  curl --fail-with-body --silent --show-error \
    --cookie "$cookie_jar" \
    --header "X-Helix-CSRF: $csrf_token" \
    --output "$health_response" \
    --write-out '%{http_code}' \
    http://127.0.0.1:8080/api/v1/health
)"
[[ "$health_status" == "200" ]] || fail "authenticated health did not return HTTP 200"
jq -e '.status == "ok" or .status == "degraded"' "$health_response" >/dev/null ||
  fail "authenticated health response had an unexpected status"

overview_status="$(
  curl --fail-with-body --silent --show-error \
    --cookie "$cookie_jar" \
    --header "X-Helix-CSRF: $csrf_token" \
    --output "$overview_response" \
    --write-out '%{http_code}' \
    http://127.0.0.1:8080/api/v1/system/overview
)"
[[ "$overview_status" == "200" ]] || fail "host overview did not return HTTP 200"
jq -e '.cpu.logical_cores >= 1 and (.storage.availability | type == "string") and (.network.availability | type == "string")' \
  "$overview_response" >/dev/null || fail "authenticated host overview was incomplete"

old_csrf_token=$csrf_token
csrf_status="$(
  curl --fail-with-body --silent --show-error \
    --cookie "$cookie_jar" \
    --header "X-Helix-CSRF: $csrf_token" \
    --header 'Content-Type: application/json' \
    --header 'Origin: http://127.0.0.1:8080' \
    --header 'Sec-Fetch-Site: same-origin' \
    --data '{}' \
    --output "$csrf_response" \
    --write-out '%{http_code}' \
    http://127.0.0.1:8080/api/v1/auth/csrf
)"
[[ "$csrf_status" == "200" ]] || fail "CSRF rotation did not return HTTP 200"
csrf_token="$(jq -er '.csrfToken' "$csrf_response")"
[[ "$csrf_token" =~ ^[A-Za-z0-9_-]{43}$ && "$csrf_token" != "$old_csrf_token" ]] ||
  fail "CSRF rotation did not return a distinct canonical proof"
stale_status="$(
  curl --silent --output "$test_root/stale-csrf.json" --write-out '%{http_code}' \
    --cookie "$cookie_jar" \
    --header "X-Helix-CSRF: $old_csrf_token" \
    http://127.0.0.1:8080/api/v1/health
)"
[[ "$stale_status" == "403" ]] || fail "the prior CSRF proof remained valid after rotation"
unset old_csrf_token

logout_status="$(
  curl --silent --show-error --output "$test_root/logout-response" --write-out '%{http_code}' \
    --cookie "$cookie_jar" \
    --cookie-jar "$cookie_jar" \
    --header "X-Helix-CSRF: $csrf_token" \
    --header 'Content-Type: application/json' \
    --header 'Origin: http://127.0.0.1:8080' \
    --header 'Sec-Fetch-Site: same-origin' \
    --data '{}' \
    http://127.0.0.1:8080/api/v1/auth/logout
)"
[[ "$logout_status" == "204" ]] || fail "logout did not return a bodyless success"
[[ ! -s "$test_root/logout-response" ]] || fail "logout unexpectedly returned a body"

jq -n --arg loginName "ci.owner" --arg password "$owner_password" \
  '{loginName:$loginName,password:$password}' >"$login_payload"
login_status="$(
  curl --fail-with-body --silent --show-error \
    --cookie-jar "$cookie_jar" \
    --header 'Content-Type: application/json' \
    --header 'Origin: http://127.0.0.1:8080' \
    --header 'Sec-Fetch-Site: same-origin' \
    --data-binary "@$login_payload" \
    --output "$login_response" \
    --write-out '%{http_code}' \
    http://127.0.0.1:8080/api/v1/auth/login
)"
[[ "$login_status" == "200" ]] || fail "password login did not return HTTP 200"
csrf_token="$(jq -er '.csrfToken' "$login_response")"
[[ "$csrf_token" =~ ^[A-Za-z0-9_-]{43}$ ]] || fail "password login did not issue a canonical CSRF proof"
health_status="$(
  curl --fail-with-body --silent --show-error \
    --cookie "$cookie_jar" \
    --header "X-Helix-CSRF: $csrf_token" \
    --output /dev/null \
    --write-out '%{http_code}' \
    http://127.0.0.1:8080/api/v1/health
)"
[[ "$health_status" == "200" ]] || fail "post-login health did not return HTTP 200"
unset csrf_token

pid_before_crash="$(systemctl show helixd.service --property=MainPID --value)"
[[ "$pid_before_crash" =~ ^[1-9][0-9]*$ ]] || fail "helixd had no live main process before crash recovery"
systemctl kill --kill-who=main --signal=SIGKILL helixd.service
restart_deadline=$((SECONDS + 30))
pid_after_crash=0
while ((SECONDS < restart_deadline)); do
  pid_after_crash="$(systemctl show helixd.service --property=MainPID --value)"
  if [[ "$pid_after_crash" =~ ^[1-9][0-9]*$ ]] &&
    [[ "$pid_after_crash" != "$pid_before_crash" ]] &&
    systemctl is-active --quiet helixd.service; then
    break
  fi
  sleep 0.2
done
[[ "$pid_after_crash" =~ ^[1-9][0-9]*$ && "$pid_after_crash" != "$pid_before_crash" ]] ||
  fail "systemd did not restart helixd after a forced crash"
runuser -u helix -- env -i PATH=/usr/bin:/bin \
  /usr/bin/helixctl --config /etc/helix/helix.toml ready --timeout-seconds 20 >/dev/null
unclean_message='an unclean Helix shutdown was detected; the full state integrity check passed'
unclean_count="$(journalctl --unit helixd.service --no-pager --output=cat | grep -F -c -- "$unclean_message" || true)"
[[ "$unclean_count" == "1" ]] || fail "forced-crash recovery did not produce exactly one integrity-check record"

systemctl stop helixd.service
systemctl is-active --quiet helixd.service && fail "helixd remained active after a graceful stop"
systemctl start helixd.service
runuser -u helix -- env -i PATH=/usr/bin:/bin \
  /usr/bin/helixctl --config /etc/helix/helix.toml ready --timeout-seconds 20 >/dev/null
unclean_count="$(journalctl --unit helixd.service --no-pager --output=cat | grep -F -c -- "$unclean_message" || true)"
[[ "$unclean_count" == "1" ]] || fail "a graceful stop was later classified as unclean"
unset pid_before_crash pid_after_crash restart_deadline unclean_count unclean_message

runuser -u helix -- env -i PATH=/usr/bin:/bin \
  /usr/bin/helixctl --config /etc/helix/helix.toml doctor --full >/dev/null
install -d -o helix -g helix -m 0700 /var/lib/helix/ci-test-backups
runuser -u helix -- env -i PATH=/usr/bin:/bin \
  /usr/bin/helixctl --config /etc/helix/helix.toml \
  backup-state /var/lib/helix/ci-test-backups/verified-state.db >/dev/null
[[ "$(stat -c '%U:%G:%a' /var/lib/helix/ci-test-backups/verified-state.db)" == "helix:helix:600" ]] ||
  fail "verified online backup mode is incorrect"
if grep -R -a -F -q -- "$owner_password" /var/lib/helix /var/cache/helix; then
  fail "owner password appeared in Helix state, backup, or cache files"
fi
if grep -R -a -F -q -- "$setup_token_canary" /var/lib/helix /var/cache/helix; then
  fail "setup token appeared in Helix state, backup, or cache files"
fi
if journalctl --unit helixd.service --no-pager --output=cat | grep -F -q -- "$owner_password"; then
  fail "owner password appeared in the service journal"
fi
if journalctl --unit helixd.service --no-pager --output=cat | grep -F -q -- "$setup_token_canary"; then
  fail "setup token appeared in the service journal"
fi
unset owner_password setup_token_canary
installation_before="$(
  runuser -u helix -- env -i PATH=/usr/bin:/bin \
    /usr/bin/helixctl --config /etc/helix/helix.toml status |
    sed -n 's/^Helix installation: //p'
)"
[[ "$installation_before" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$ ]] ||
  fail "installed state had no canonical UUIDv4 installation identity"

daemon_hash_before="$(sha256sum /usr/bin/helixd | sed 's/[[:space:]].*$//')"
tampered_bundle="$test_root/tampered-bundle"
cp -a -- "$bundle_dir" "$tampered_bundle"
printf '\n<!-- intentional CI tamper -->\n' >>"$tampered_bundle/web/index.html"
if "$tampered_bundle/install-local.sh" --bundle "$tampered_bundle" >"$test_root/tampered.log" 2>&1; then
  fail "a bundle with a modified payload passed manifest verification"
fi
systemctl is-active --quiet helixd.service || fail "tampered-bundle rejection disturbed the running service"
[[ "$(sha256sum /usr/bin/helixd | sed 's/[[:space:]].*$//')" == "$daemon_hash_before" ]] ||
  fail "tampered-bundle rejection changed the installed daemon"

reinstall_log="$test_root/reinstall.log"
"$bundle_dir/install-local.sh" --bundle "$bundle_dir" >"$reinstall_log" 2>&1 || fail "repeat installation failed"
if grep -q '^Owner setup token (shown once):$' "$reinstall_log"; then
  fail "repeat installation replaced the owner bootstrap token"
fi
installation_after="$(
  runuser -u helix -- env -i PATH=/usr/bin:/bin \
    /usr/bin/helixctl --config /etc/helix/helix.toml status |
    sed -n 's/^Helix installation: //p'
)"
[[ "$installation_after" == "$installation_before" ]] || fail "repeat installation replaced critical state"

snapshot_id="$(sed -n 's/^Package-file rollback snapshot: //p' "$reinstall_log" | tail -n 1)"
[[ "$snapshot_id" =~ ^snapshot-[0-9]{8}T[0-9]{6}Z-[A-Za-z0-9]{8}$ ]] ||
  fail "repeat installation did not report a canonical rollback snapshot"
"$bundle_dir/rollback-local.sh" --snapshot "$snapshot_id" >"$test_root/rollback.log" 2>&1 ||
  fail "explicit package rollback failed"
systemctl is-active --quiet helixd.service || fail "service was not restored after package rollback"
systemctl is-enabled --quiet helixd.service || fail "service enablement was not restored after package rollback"
runuser -u helix -- env -i PATH=/usr/bin:/bin \
  /usr/bin/helixctl --config /etc/helix/helix.toml ready --timeout-seconds 20 >/dev/null

probe_root="$(mktemp -d /run/helix-package-probe.XXXXXXXX)"
[[ "$probe_root" == /run/helix-package-probe.* && -d "$probe_root" && ! -L "$probe_root" ]] ||
  fail "could not create a safe preserved-state probe root"
chmod 0755 -- "$probe_root"
install -o root -g root -m 0755 -- "$bundle_dir/bin/helixctl" "$probe_root/helixctl"
cmp -s -- /usr/bin/helixctl "$probe_root/helixctl" ||
  fail "preserved-state probe does not match the installed CLI"

"$bundle_dir/uninstall-local.sh" >"$test_root/uninstall.log" 2>&1 || fail "data-preserving uninstall failed"
if systemctl is-active --quiet helixd.service; then
  fail "helixd remained active after uninstall"
fi
if systemctl is-enabled --quiet helixd.service; then
  fail "helixd remained enabled after uninstall"
fi
for removed in \
  /usr/bin/helixd \
  /usr/bin/helixctl \
  /usr/share/helix \
  /usr/lib/systemd/system/helixd.service \
  /usr/lib/sysusers.d/helix.conf \
  /usr/lib/tmpfiles.d/helix.conf; do
  [[ ! -e "$removed" && ! -L "$removed" ]] || fail "uninstall left a package-owned path: $removed"
done
[[ -f /etc/helix/helix.toml ]] || fail "uninstall removed administrator configuration"
[[ -f /var/lib/helix/state/helix-state.db ]] || fail "uninstall removed critical state"
getent passwd helix >/dev/null || fail "uninstall removed the service account"
getent group helix >/dev/null || fail "uninstall removed the service group"

preserved_installation="$(
  runuser -u helix -- env -i PATH=/usr/bin:/bin \
    "$probe_root/helixctl" --config /etc/helix/helix.toml status |
    sed -n 's/^Helix installation: //p'
)"
[[ "$preserved_installation" == "$installation_before" ]] || fail "uninstall did not preserve readable critical state"

printf 'Linux package lifecycle passed: install, owner claim, protected API, CSRF rotation, logout/login, full doctor, verified backup, modified-bundle manifest rejection, repeat install, rollback, and data-preserving uninstall.\n'
