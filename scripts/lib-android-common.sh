# Shared Android SDK helpers for setup-android-sdk.sh and
# ci-boot-android-emulator.sh. Source-only: defines constants and
# functions, changes no shell state. Source it as:
#   SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
#   source "$SCRIPT_DIR/lib-android-common.sh"

readonly AVD_NAME="agent-mobile-api37"
readonly CMDTOOLS_BUILD="15859902"

fail() {
    echo "error: $1" >&2
    if [ "${2:-}" != "" ]; then
        echo "next: $2" >&2
    fi
    exit 1
}

# SDK root from the environment; setup-android-sdk.sh pre-exports its
# platform default before sourcing, so a missing value here means the
# SDK was never installed.
sdk_root() {
    local root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
    [ -n "$root" ] || fail "ANDROID_HOME/ANDROID_SDK_ROOT unset" \
        "run scripts/setup-android-sdk.sh, then export its printed variables"
    echo "$root"
}

adb_bin() { echo "$(sdk_root)/platform-tools/adb"; }
emulator_bin() { echo "$(sdk_root)/emulator/emulator"; }

# Pinned cmdline-tools build only — never a directory glob, so a stale
# alternate install can never shadow the revision setup installed.
avdmanager_bin() {
    local b
    b="$(sdk_root)/cmdline-tools/$CMDTOOLS_BUILD/bin/avdmanager"
    [ -x "$b" ] && { echo "$b"; return; }
    return 1
}
