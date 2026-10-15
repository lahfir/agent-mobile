#!/usr/bin/env bash
# CI emulator lifecycle for the agent-mobile Android gate.
#
# Usage: scripts/ci-boot-android-emulator.sh [start|diagnostics|stop]
#   start        boot the pinned AVD headless; print GITHUB_ENV lines only
#   diagnostics  bounded, redacted logs for a failed run (never fails)
#   stop         kill exactly the recorded emulator; optionally delete the AVD
#
# Required env: ANDROID_HOME or ANDROID_SDK_ROOT, and ANDROID_AVD_HOME.
# `start` also requires GNU coreutils `timeout` for bounded probes.
set -euo pipefail
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib-android-common.sh"

readonly TEMP_ROOT="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
readonly STATE_DIR="$TEMP_ROOT/agent-mobile-android-ci"
readonly PID_FILE="$STATE_DIR/emulator.pid"
readonly SERIAL_FILE="$STATE_DIR/serial"
readonly EMU_LOG="$STATE_DIR/emulator.log"
readonly BOOT_BUDGET_SECS=600
readonly ADB_TIMEOUT=15
readonly EMU_TIMEOUT=30

# Bounded probes: every adb call gets 15s, every emulator/avdmanager call
# 30s — a single wedged call must never consume the whole boot budget.
adb_t() { timeout "$ADB_TIMEOUT" "$(adb_bin)" "$@"; }
emu_t() { timeout "$EMU_TIMEOUT" "$(emulator_bin)" "$@"; }

# Strict identity probe: succeeds only when `emu avd name` exits zero and
# its first line is a nonempty name (never `OK`); prints that CR-trimmed
# first line. Any command failure or empty/OK-only reply is exit 1 — the
# caller must decide match/no-match itself; this helper never guesses.
probe_avd_name() {
    local serial="$1" out first
    out=$(adb_t -s "$serial" emu avd name 2>/dev/null) || return 1
    first=${out%%$'\n'*}
    first=${first%$'\r'}
    { [ -n "$first" ] && [ "$first" != "OK" ]; } || return 1
    printf '%s\n' "$first"
}

valid_pid() { [[ "${1:-}" =~ ^[0-9]+$ ]] && [ "$1" -gt 1 ]; }
valid_serial() { [[ "${1:-}" =~ ^emulator-[0-9]+$ ]]; }

redact() {
    sed -E \
        -e 's/Authorization:[[:space:]]*Bearer[[:space:]]+[^[:space:]]+/Authorization: Bearer <redacted>/Ig' \
        -e 's/[A-Za-z0-9_-]{43}/<redacted-token>/g' \
        -e 's/[a-f0-9]{24}/<redacted-hex>/g'
}

check_accel() {
    emu_t -accel-check >/dev/null 2>&1 || \
        fail "emulator acceleration unavailable" "enable KVM/hypervisor"
    [ "$(uname -s)" = "Linux" ] || return 0
    [ -w /dev/kvm ] || fail "/dev/kvm absent or not writable" \
        "enable KVM on the host (CI: sudo chmod a+rw /dev/kvm)"
}

avd_present() {
    emu_t -list-avds 2>/dev/null | grep -qxF "$AVD_NAME"
}

ensure_console_token() {
    local tok="$HOME/.emulator_console_auth_token"
    [ -s "$tok" ] && return 0
    python3 - "$tok" <<'PY' || fail "cannot mint emulator console auth token" "python3 required"
import secrets, sys
with open(sys.argv[1], "w") as f:
    f.write(secrets.token_hex(12))
PY
    chmod 600 "$tok"
}

# Resolved state-dir path must keep the exact basename, sit strictly
# under the resolved temp root, and never be a symlink — shared by the
# start-time creation check and the stop-time removal check.
validate_state_path() {
    local real expected_root
    real=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$STATE_DIR")
    expected_root=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$TEMP_ROOT")
    [ "$(basename "$real")" = "agent-mobile-android-ci" ] || return 1
    [ ! -L "$STATE_DIR" ] || return 1
    case "$real" in
        "$expected_root"/agent-mobile-android-ci) return 0 ;;
        *) return 1 ;;
    esac
}

# Create the private state dir, then prove it is the path we think it is
# before any file lands in it.
ensure_state_dir() {
    [ ! -L "$STATE_DIR" ] ||         fail "state dir $STATE_DIR is a symlink" "remove it by hand"
    mkdir -p "$STATE_DIR" || fail "cannot create $STATE_DIR" "check permissions"
    validate_state_path || fail "state dir $STATE_DIR fails the ownership check"         "pick a different RUNNER_TEMP/TMPDIR"
}

# Does the recorded pid still identify as OUR emulator? The command must
# show qemu/emulator evidence AND the exact `-avd <name>` argument — PID
# reuse or a different AVD can never steer a signal at something else.
pid_is_our_emulator() {
    local pid="$1" cmd
    valid_pid "$pid" || return 1
    kill -0 "$pid" 2>/dev/null || return 1
    cmd=$(ps -o command= -p "$pid" 2>/dev/null || true)
    case "$cmd" in
        *emulator*|*qemu*) ;;
        *) return 1 ;;
    esac
    [[ "$cmd" == *"-avd $AVD_NAME"* ]]
}

# One bounded correlation pass over every live `emulator-*` row: prints
# the single serial whose `emu avd name` is exactly ours; exits 1 when
# zero or more than one matches.
unique_avd_serial() {
    local row cand state name found=0 serial="" devices
    devices=$(adb_t devices 2>/dev/null) || return 2
    while IFS= read -r row; do
        read -r cand state _ <<< "$row"
        case "$cand" in
            emulator-*) ;;
            *) continue ;;
        esac
        [ "$state" = "device" ] || continue
        name=$(probe_avd_name "$cand") || return 2
        if [ "$name" = "$AVD_NAME" ]; then
            found=$((found + 1))
            serial="$cand"
        fi
    done <<< "$devices"
    [ "$found" -eq 1 ] || return 1
    echo "$serial"
}

# Before spawn: every live emulator row must answer `emu avd name`, and
# none may already claim our exact AVD — CI only owns what it starts.
reject_existing_avd() {
    local devices row cand state name
    devices=$(adb_t devices) ||         fail "adb devices failed before spawn" "check the adb server health"
    while IFS= read -r row; do
        read -r cand state _ <<< "$row"
        case "$cand" in
            emulator-*) ;;
            *) continue ;;
        esac
        [ "$state" = "device" ] || continue
        name=$(probe_avd_name "$cand") ||             fail "cannot prove AVD identity for $cand" \
                "check \`adb -s $cand emu avd name\` and retry"
        [ "$name" = "$AVD_NAME" ] &&             fail "an emulator already runs $AVD_NAME on $cand" \
                "CI never adopts a foreign emulator; stop it or pick a new AVD"
    done <<< "$devices"
}

# Every emulator-<n> row in state `device` gets a bounded `emu avd name`
# probe; exactly one match for our AVD is required before boot_completed.
wait_for_boot() {
    local deadline serial row cand state name matches booted
    deadline=$((SECONDS + BOOT_BUDGET_SECS))
    while [ "$SECONDS" -lt "$deadline" ]; do
        matches=""
        local devices
        devices=$(adb_t devices) ||             { echo "adb devices failed during boot wait" >&2; return 1; }
        while IFS= read -r row; do
            read -r cand state _ <<< "$row"
            case "$cand" in
                emulator-*) ;;
                *) continue ;;
            esac
            [ "$state" = "device" ] || continue
            name=$(probe_avd_name "$cand") || {
                echo "cannot prove AVD identity for $cand" >&2
                return 1
            }
            [ "$name" = "$AVD_NAME" ] && matches="$matches $cand"
        done <<< "$devices"
        # shellcheck disable=SC2086
        set -- $matches
        if [ "$#" -gt 1 ]; then
            echo "multiple emulators claim AVD $AVD_NAME:$matches" >&2
            return 1
        fi
        if [ "$#" -eq 1 ]; then
            serial="$1"
            echo "$serial" > "$SERIAL_FILE"
            booted=$(adb_t -s "$serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '[:space:]' || true)
            [ "$booted" = "1" ] && return 0
        fi
        sleep 2
    done
    return 1
}

cmd_start() {
    command -v timeout >/dev/null 2>&1 || \
        fail "GNU timeout not found" "install coreutils for bounded probes"
    ensure_state_dir
    check_accel
    [ -n "${ANDROID_AVD_HOME:-}" ] || fail "ANDROID_AVD_HOME unset" \
        "point it at the directory holding $AVD_NAME.avd"
    avd_present || fail "AVD $AVD_NAME not in emulator -list-avds" \
        "run scripts/setup-android-sdk.sh to create it"
    ensure_console_token
    if [ -f "$PID_FILE" ]; then
        local oldpid
        oldpid=$(cat "$PID_FILE" 2>/dev/null || true)
        if valid_pid "$oldpid"; then
            kill -0 "$oldpid" 2>/dev/null && \
                fail "recorded emulator pid $oldpid still alive" \
                    "run '$0 stop' first"
            rm -f "$PID_FILE" "$SERIAL_FILE" 2>/dev/null || true
        else
            fail "pid file $PID_FILE is corrupt" \
                "remove $STATE_DIR by hand after checking; refusing to signal anything"
        fi
    fi
    rm -f "$SERIAL_FILE" 2>/dev/null || true
    reject_existing_avd
    echo "starting headless AVD $AVD_NAME (log: $EMU_LOG)" >&2
    "$(emulator_bin)" -avd "$AVD_NAME" \
        -no-window -no-audio -no-boot-anim -no-snapshot-load \
        -no-snapshot-save -gpu swiftshader_indirect \
        > "$EMU_LOG" 2>&1 &
    echo $! > "$PID_FILE"
    if ! wait_for_boot; then
        echo "emulator failed to boot within ${BOOT_BUDGET_SECS}s" >&2
        tail -60 "$EMU_LOG" | redact >&2 || true
        exit 1
    fi
    local serial
    serial=$(cat "$SERIAL_FILE")
    echo "emulator ready on $serial" >&2
    echo "ANDROID_SERIAL=$serial"
    echo "AGENT_MOBILE_TEST_ANDROID_DEVICE=android:avd:$AVD_NAME"
}

cmd_diagnostics() {
    set +e
    local adb roots=() f
    adb="$(adb_bin 2>/dev/null)"
    if [ -n "$adb" ] && [ -x "$adb" ]; then
        echo "== adb devices =="
        timeout "$ADB_TIMEOUT" "$adb" devices 2>&1 | redact
        echo "== adb forward --list =="
        timeout "$ADB_TIMEOUT" "$adb" forward --list 2>&1 | redact
    fi
    [ -f "$EMU_LOG" ] && { echo "== emulator log tail =="; tail -200 "$EMU_LOG" | redact; }
    if [ -f "$SERIAL_FILE" ] && [ -n "$adb" ] && [ -x "$adb" ]; then
        local serial
        serial=$(cat "$SERIAL_FILE")
        if valid_serial "$serial"; then
            echo "== logcat tail ($serial) =="
            timeout "$ADB_TIMEOUT" "$adb" -s "$serial" logcat -d -t 300 2>&1 | redact
        fi
    fi
    echo "== driver logs tail =="
    roots=("${TMPDIR:-/tmp}")
    [ -n "${RUNNER_TEMP:-}" ] && [ "$RUNNER_TEMP" != "${TMPDIR:-/tmp}" ] && \
        roots+=("$RUNNER_TEMP")
    local root
    for root in "${roots[@]}"; do
        find "$root" -maxdepth 3 -name 'driver-*.log' -mmin -60 2>/dev/null | \
            while IFS= read -r f; do
                echo "--- $f ---"
                tail -120 "$f" | redact
            done
    done
    echo "== android lint text =="
    local n=0
    shopt -s nullglob
    for f in drivers/android/app/build/reports/lint-results*.txt; do
        [ "$n" -ge 4 ] && break
        echo "--- $f ---"; tail -120 "$f" | redact
        n=$((n + 1))
    done
    shopt -u nullglob
    set -e
    return 0
}

cmd_stop() {
    local serial="" pid="" adb="" name
    [ -f "$PID_FILE" ] && pid=$(cat "$PID_FILE")
    [ -f "$SERIAL_FILE" ] && serial=$(cat "$SERIAL_FILE")
    [ -z "$pid" ] || valid_pid "$pid" || \
        fail "pid file content $pid is not a safe pid" "inspect $PID_FILE"
    [ -z "$serial" ] || valid_serial "$serial" || \
        fail "serial file content $serial is not an emulator serial" "inspect $SERIAL_FILE"
    adb="$(adb_bin 2>/dev/null || true)"
    if valid_serial "${serial:-}" && { [ -z "$adb" ] || [ ! -x "$adb" ]; }; then
        fail "adb unavailable for the recorded serial $serial" \
            "restore the SDK/adb path; state is retained"
    fi
    # A valid PID without a serial means start died before boot_completed
    # wrote it — start already rejected pre-existing same-name emulators,
    # so a unique exact-name AVD correlation is provably ours.
    if [ -n "$pid" ] && [ -z "$serial" ] && [ -n "$adb" ] && [ -x "$adb" ]; then
        local rc=0
        serial=$(unique_avd_serial 2>/dev/null) || rc=$?
        if [ "$rc" -eq 2 ]; then
            fail "cannot prove AVD identity for a live emulator row"                 "check adb server and \`emu avd name\` probes, then retry stop"
        fi
    fi
    # Kill by adb only when ALL ownership links hold: recorded PID alive
    # AND still an emulator running our AVD, recorded serial in `device`
    # state, and its console name exactly ours. Any lost link skips the
    # adb kill — the live-serial re-check below still fails closed.
    if pid_is_our_emulator "${pid:-}" && valid_serial "$serial" && \
        [ -n "$adb" ] && [ -x "$adb" ] && \
        timeout "$ADB_TIMEOUT" "$adb" devices 2>/dev/null | \
        awk '{print $1, $2}' | grep -qx "$serial device"; then
        if name=$(probe_avd_name "$serial"); then
            if [ "$name" = "$AVD_NAME" ]; then
                timeout "$ADB_TIMEOUT" "$adb" -s "$serial" emu kill >/dev/null 2>&1 || true
            else
                echo "serial $serial reports AVD '$name', not $AVD_NAME — skipping emu kill" >&2
            fi
        else
            echo "cannot prove AVD identity for $serial — skipping emu kill" >&2
        fi
    elif [ -n "${serial:-}" ] && valid_serial "$serial"; then
        echo "skipping emu kill: recorded pid ${pid:-none} not proven to be $AVD_NAME" >&2
    fi
    if [ -n "$pid" ]; then
        local waited=0
        while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 30 ]; do
            sleep 1; waited=$((waited + 1))
        done
        if kill -0 "$pid" 2>/dev/null; then
            pid_is_our_emulator "$pid" || \
                fail "pid $pid is not an emulator running $AVD_NAME" \
                    "refusing to signal a foreign process; inspect $PID_FILE"
            kill -TERM "$pid" 2>/dev/null || true
            waited=0
            while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 30 ]; do
                sleep 1; waited=$((waited + 1))
            done
            if kill -0 "$pid" 2>/dev/null; then
                pid_is_our_emulator "$pid" || \
                    fail "pid $pid identity changed before KILL" "inspect manually"
                kill -KILL "$pid" 2>/dev/null || true
                sleep 1
                kill -0 "$pid" 2>/dev/null && \
                    fail "pid $pid survived KILL" "investigate the emulator process"
            fi
        fi
    fi
    # If the owned serial still answers as OUR booted AVD, stop failed —
    # never hide a live emulator behind a successful stop.
    if [ -n "$adb" ] && [ -x "$adb" ] && valid_serial "${serial:-}"; then
        local devices_now
        devices_now=$(timeout "$ADB_TIMEOUT" "$adb" devices 2>/dev/null) || \
            fail "adb devices failed during the final stop proof" \
                "check the adb server; state is retained"
        if printf '%s\n' "$devices_now" | \
            awk '{print $1, $2}' | grep -qx "$serial device"; then
            name=$(probe_avd_name "$serial") || \
                fail "cannot prove AVD identity for $serial" \
                    "check \`adb -s $serial emu avd name\`; the row is still live"
            [ "$name" = "$AVD_NAME" ] && \
                fail "emulator $serial still runs $AVD_NAME" \
                    "kill it with: adb -s $serial emu kill"
        fi
    fi
    if [ "${CI_ANDROID_DELETE_AVD:-}" = "1" ]; then
        delete_avd
    fi
    remove_state_dir
}

# Delete the exact pinned AVD — only when its home resolves strictly
# beneath RUNNER_TEMP — and surface avdmanager failure unless the AVD is
# verifiably gone.
delete_avd() {
    [ -n "${RUNNER_TEMP:-}" ] || \
        fail "RUNNER_TEMP unset" "refusing to delete AVD outside a runner temp root"
    local avd_home_real temp_real
    avd_home_real=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' \
        "${ANDROID_AVD_HOME:-/nonexistent}")
    temp_real=$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$RUNNER_TEMP")
    case "$avd_home_real" in
        "$temp_real"/*) ;;
        *) fail "AVD home $avd_home_real not under $temp_real" \
            "refusing deletion outside the runner temp root" ;;
    esac
    local avdm
    avdm=$(avdmanager_bin) || fail "avdmanager not found" "check the SDK layout"
    if timeout "$EMU_TIMEOUT" "$avdm" delete avd -n "$AVD_NAME" >/dev/null 2>&1; then
        return 0
    fi
    # Acceptable only when the exact AVD artifacts are already absent.
    if [ ! -e "$ANDROID_AVD_HOME/$AVD_NAME.avd" ] && \
        [ ! -e "$ANDROID_AVD_HOME/$AVD_NAME.ini" ]; then
        return 0
    fi
    fail "avdmanager could not delete $AVD_NAME" \
        "remove $ANDROID_AVD_HOME/$AVD_NAME.avd and .ini by hand"
}

# Remove only our state dir once the shared ownership predicate passes.
remove_state_dir() {
    [ -d "$STATE_DIR" ] || [ -L "$STATE_DIR" ] || return 0
    validate_state_path || fail "state dir $STATE_DIR fails the ownership check" \
        "remove it by hand"
    rm -r -- "$STATE_DIR" || fail "could not remove $STATE_DIR" "remove it by hand"
}

main() {
    case "${1:-start}" in
        start) cmd_start ;;
        diagnostics) cmd_diagnostics ;;
        stop) cmd_stop ;;
        *) fail "usage: $0 [start|diagnostics|stop]" "" ;;
    esac
}

main "$@"
