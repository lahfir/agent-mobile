#!/usr/bin/env bash
set -euo pipefail

readonly CMDTOOLS_BUILD="15859902"
readonly API_LEVEL="37"
readonly PLATFORM_ID="android-${API_LEVEL}.0"
readonly BUILD_TOOLS_VERSION="36.0.0"
# Exact published revisions: cmdline-tools build 15859902 ships
# Pkg.Revision 22.0; the others are the required API-37 package revisions
# on both arm64-v8a and x86_64.
readonly CMDTOOLS_REVISION="22.0"
readonly PLATFORM_TOOLS_REVISION="37.0.1"
readonly PLATFORM_REVISION="2"
readonly BUILD_TOOLS_REVISION="36.0.0"
readonly EMULATOR_REVISION="37.2.12"
readonly SYS_IMAGE_REVISION="6"
readonly AVD_NAME="agent-mobile-api37"
readonly AVD_DEVICE="pixel_7"
readonly DOWNLOAD_BASE="https://dl.google.com/android/repository"
readonly REQUIRED_JDK_MAJOR="17"

# Host default SDK root: Darwin uses ~/Library/Android/sdk and Linux uses
# ~/Android/Sdk; the `*` arm only exists because detect_host immediately
# rejects unsupported hosts anyway.
default_sdk_dir() {
    case "$(uname -s)" in
        Darwin) echo "$HOME/Library/Android/sdk" ;;
        Linux) echo "$HOME/Android/Sdk" ;;
        *) echo "$HOME/Android/Sdk" ;;
    esac
}

ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$(default_sdk_dir)}}"
readonly ANDROID_HOME

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
readonly REPO_ROOT

HOST_OS=""
HOST_ARCH=""
CMDTOOLS_ZIP=""
CMDTOOLS_SHA256=""
SYS_IMAGE_ABI=""

TMP_WORK=""
STAGING_DIR=""
cleanup() {
    case "${TMP_WORK:-}" in
        "${TMPDIR:-/tmp}/agent-mobile-sdk."*) rm -r -- "$TMP_WORK" 2>/dev/null || true ;;
    esac
    case "${STAGING_DIR:-}" in
        "${ANDROID_HOME}/.staging-cmdline-tools-${CMDTOOLS_BUILD}."*) rm -r -- "$STAGING_DIR" 2>/dev/null || true ;;
    esac
}
trap cleanup EXIT

fail() {
    echo "error: $1" >&2
    if [ "${2:-}" != "" ]; then
        echo "next: $2" >&2
    fi
    exit 1
}

usage() {
    echo "usage: $0 [--check]"
    echo "  (no args)  install the pinned headless Android SDK pieces"
    echo "  --check    verify tools, packages, AVD, acceleration, and Studio absence"
}

detect_host() {
    HOST_OS="$(uname -s)"
    HOST_ARCH="$(uname -m)"
    case "${HOST_OS}:${HOST_ARCH}" in
        Darwin:arm64)
            CMDTOOLS_ZIP="commandlinetools-mac_arm64-${CMDTOOLS_BUILD}_latest.zip"
            CMDTOOLS_SHA256="835b62a26162b229b441d1f6d4680383815a270809eb33522c0d480fa5002c4e"
            SYS_IMAGE_ABI="arm64-v8a"
            ;;
        Darwin:x86_64)
            CMDTOOLS_ZIP="commandlinetools-mac_x86_64-${CMDTOOLS_BUILD}_latest.zip"
            CMDTOOLS_SHA256="c5a6378ab5cf7e0d5701921405115befff13e9ff7417fb588389338f8bd050f3"
            SYS_IMAGE_ABI="x86_64"
            ;;
        Linux:x86_64)
            CMDTOOLS_ZIP="commandlinetools-linux-${CMDTOOLS_BUILD}_latest.zip"
            CMDTOOLS_SHA256="4e4c464f145a7512b57d088ac6c278c03c9eea610886b35a5e0804e74eedf583"
            SYS_IMAGE_ABI="x86_64"
            ;;
        *)
            fail "unsupported host ${HOST_OS}/${HOST_ARCH}" \
                "supported hosts: macOS arm64, macOS x86_64, Linux x86_64"
            ;;
    esac
}

sdkmanager_bin() { echo "${ANDROID_HOME}/cmdline-tools/${CMDTOOLS_BUILD}/bin/sdkmanager"; }
avdmanager_bin() { echo "${ANDROID_HOME}/cmdline-tools/${CMDTOOLS_BUILD}/bin/avdmanager"; }
emulator_bin() { echo "${ANDROID_HOME}/emulator/emulator"; }
adb_bin() { echo "${ANDROID_HOME}/platform-tools/adb"; }
avd_home() { echo "${ANDROID_AVD_HOME:-$HOME/.android/avd}"; }
local_properties_file() { echo "${REPO_ROOT}/drivers/android/local.properties"; }

jdk_major() {
    local out version
    out="$(javac -version 2>&1)" || return 1
    version="${out#javac }"
    version="${version%% *}"
    case "$version" in
        1.*) version="${version#1.}" ;;
    esac
    echo "${version%%.*}"
}

require_jdk17() {
    local major
    if ! major="$(jdk_major)" || [ "$major" != "$REQUIRED_JDK_MAJOR" ]; then
        fail "JDK ${REQUIRED_JDK_MAJOR} is required (javac not found or wrong major version)" \
            "install a JDK ${REQUIRED_JDK_MAJOR} and ensure javac is on PATH"
    fi
}

sha256_file() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        fail "no SHA-256 tool available" "install shasum or sha256sum"
    fi
}

dir_nonempty() {
    [ -d "$1" ] && [ -n "$(ls -A "$1" 2>/dev/null)" ]
}

run_with_deadline() {
    local secs="$1"; shift
    python3 "$SCRIPT_DIR/run_with_deadline.py" "$secs" "$@"
}

cmdline_tools_revision() {
    awk -F= '/^Pkg.Revision[[:space:]]*=/ { sub(/[[:space:]]+$/, "", $2); print $2; exit }' \
        "$1/source.properties" 2>/dev/null
}

install_cmdline_tools() {
    local sdkmanager
    sdkmanager="$(sdkmanager_bin)"
    if [ -x "$sdkmanager" ]; then
        local rev
        rev="$(cmdline_tools_revision "${ANDROID_HOME}/cmdline-tools/${CMDTOOLS_BUILD}")"
        [ "$rev" = "$CMDTOOLS_REVISION" ] || fail \
            "command-line tools revision mismatch: ${rev:-unknown} != ${CMDTOOLS_REVISION}" \
            "remove ${ANDROID_HOME}/cmdline-tools/${CMDTOOLS_BUILD} and re-run"
        echo "command-line tools ${CMDTOOLS_BUILD} (${rev}): already installed"
        return
    fi
    local dest="${ANDROID_HOME}/cmdline-tools/${CMDTOOLS_BUILD}"
    if [ -e "$dest" ]; then
        fail "${dest} exists but sdkmanager is missing" \
            "inspect or move aside ${dest}, then re-run"
    fi

    command -v curl >/dev/null 2>&1 || fail "curl is required" "install curl"
    command -v unzip >/dev/null 2>&1 || fail "unzip is required" "install unzip"

    TMP_WORK="$(mktemp -d "${TMPDIR:-/tmp}/agent-mobile-sdk.XXXXXX")"
    local zip_path="${TMP_WORK}/${CMDTOOLS_ZIP}"
    local url="${DOWNLOAD_BASE}/${CMDTOOLS_ZIP}"
    echo "downloading ${url}"
    curl --fail --silent --show-error --location \
        --connect-timeout 20 --max-time 600 --output "$zip_path" "$url" \
        || fail "download failed: ${url}" "check network access to dl.google.com"

    local actual_sha
    actual_sha="$(sha256_file "$zip_path")"
    if [ "$actual_sha" != "$CMDTOOLS_SHA256" ]; then
        fail "checksum mismatch for ${CMDTOOLS_ZIP}" \
            "expected ${CMDTOOLS_SHA256}, got ${actual_sha}; refusing to extract"
    fi

    unzip -q "$zip_path" -d "${TMP_WORK}/extract"
    if [ ! -d "${TMP_WORK}/extract/cmdline-tools/bin" ]; then
        fail "unexpected archive layout in ${CMDTOOLS_ZIP}" "report the archive contents"
    fi

    mkdir -p "${ANDROID_HOME}/cmdline-tools"
    STAGING_DIR="$(mktemp -d "${ANDROID_HOME}/.staging-cmdline-tools-${CMDTOOLS_BUILD}.XXXXXX")"
    mv "${TMP_WORK}/extract/cmdline-tools" "${STAGING_DIR}/cmdline-tools"
    if [ -e "$dest" ]; then
        fail "${dest} appeared while staging" "inspect ${dest} and re-run"
    fi
    local staged_rev
    staged_rev="$(cmdline_tools_revision "${STAGING_DIR}/cmdline-tools")"
    [ "$staged_rev" = "$CMDTOOLS_REVISION" ] || fail \
        "downloaded command-line tools report ${staged_rev:-unknown}, expected ${CMDTOOLS_REVISION}" \
        "re-run; if it persists the pinned archive changed"
    mv "${STAGING_DIR}/cmdline-tools" "$dest"
    rmdir "$STAGING_DIR"
    STAGING_DIR=""
    rm -r -- "$TMP_WORK"
    TMP_WORK=""
    echo "command-line tools ${CMDTOOLS_BUILD}: installed to ${dest}"
}

# `sdkmanager --list_installed` / `--list` → exact `id<TAB>revision` rows.
sdkmanager_package_rows() {
    run_with_deadline 60 "$(sdkmanager_bin)" --sdk_root="$ANDROID_HOME" "$@" 2>/dev/null \
        | awk -F'|' '{ id = $1; rev = $2
                      sub(/^[ \t]+/, "", id); sub(/[ \t]+$/, "", id)
                      sub(/^[ \t]+/, "", rev); sub(/[ \t]+$/, "", rev)
                      if (id ~ /[a-zA-Z]/ && id != "Path") print id "\t" rev }'
}

installed_package_rows() { sdkmanager_package_rows --list_installed; }

available_package_rows() { sdkmanager_package_rows --list; }

# Exact `id` row's revision; empty when the id is absent.
package_revision() {
    printf '%s\n' "$1" | awk -F'\t' -v want="$2" '$1 == want { print $2; exit }'
}

install_packages() {
    local sdkmanager
    sdkmanager="$(sdkmanager_bin)"
    local required=(
        "platform-tools"
        "platforms;${PLATFORM_ID}"
        "build-tools;${BUILD_TOOLS_VERSION}"
        "emulator"
        "system-images;${PLATFORM_ID};google_apis;${SYS_IMAGE_ABI}"
    )
    local revisions=(
        "$PLATFORM_TOOLS_REVISION"
        "$PLATFORM_REVISION"
        "$BUILD_TOOLS_REVISION"
        "$EMULATOR_REVISION"
        "$SYS_IMAGE_REVISION"
    )
    local installed
    installed="$(installed_package_rows)"
    local missing=()
    local i id rev want
    for i in "${!required[@]}"; do
        id="${required[$i]}"
        want="${revisions[$i]}"
        rev="$(package_revision "$installed" "$id")"
        if [ -z "$rev" ]; then
            missing+=("$id")
        elif [ "$rev" != "$want" ]; then
            fail "SDK package ${id} is ${rev}, required ${want}" \
                "reconcile ${id} manually to ${want}; setup never auto-upgrades or downgrades"
        fi
    done
    if [ "${#missing[@]}" -eq 0 ]; then
        echo "SDK packages: all required packages installed at exact revisions"
        return
    fi
    local available advertised
    available="$(available_package_rows)"
    for id in "${missing[@]}"; do
        for i in "${!required[@]}"; do
            [ "${required[$i]}" = "$id" ] && want="${revisions[$i]}"
        done
        advertised="$(package_revision "$available" "$id")"
        [ "$advertised" = "$want" ] || fail \
            "SDK package ${id} advertised as ${advertised:-absent}, required ${want}" \
            "the pinned revision is unavailable; reconcile the repository or the pin explicitly"
    done
    echo "installing missing SDK packages: ${missing[*]}"
    printf 'y\n%.0s' $(seq 1 256) | run_with_deadline 300 "$sdkmanager" --sdk_root="$ANDROID_HOME" --licenses >/dev/null
    run_with_deadline 1200 "$sdkmanager" --sdk_root="$ANDROID_HOME" "${missing[@]}"
    installed="$(installed_package_rows)"
    for i in "${!required[@]}"; do
        id="${required[$i]}"
        rev="$(package_revision "$installed" "$id")"
        [ "$rev" = "${revisions[$i]}" ] || fail \
            "SDK package ${id} reported ${rev:-missing} after install, required ${revisions[$i]}" \
            "inspect the sdkmanager output and reconcile ${id} manually"
    done
}

avd_exists() {
    local emu
    emu="$(emulator_bin)"
    [ -x "$emu" ] || return 1
    run_with_deadline 30 "$emu" -list-avds 2>/dev/null | grep -qxF "$AVD_NAME"
}

avd_matches_pins() {
    local ini
    ini="$(avd_home)/${AVD_NAME}.avd/config.ini"
    [ -f "$ini" ] || return 1
    grep -qxF "abi.type=${SYS_IMAGE_ABI}" "$ini" \
        && grep -qxF "hw.device.name=${AVD_DEVICE}" "$ini" \
        && grep -qxF "image.sysdir.1=system-images/${PLATFORM_ID}/google_apis/${SYS_IMAGE_ABI}/" "$ini" \
        && grep -qxF "tag.id=google_apis" "$ini" \
        && grep -qxF "target=${PLATFORM_ID}" "$ini"
}

create_avd() {
    if avd_exists; then
        if avd_matches_pins; then
            echo "AVD ${AVD_NAME}: already exists"
            return
        fi
        fail "AVD ${AVD_NAME} exists but does not match the pinned config" \
            "inspect $(avd_home)/${AVD_NAME}.avd/config.ini; move aside or delete only \
             the named AVD, then re-run"
    fi
    local avdmanager
    avdmanager="$(avdmanager_bin)"
    echo "creating AVD ${AVD_NAME} (${AVD_DEVICE}, ${PLATFORM_ID}/${SYS_IMAGE_ABI})"
    printf 'no\n' | run_with_deadline 120 env "ANDROID_SDK_ROOT=$ANDROID_HOME" "ANDROID_HOME=$ANDROID_HOME" \
        "$avdmanager" create avd \
        --name "$AVD_NAME" \
        --package "system-images;${PLATFORM_ID};google_apis;${SYS_IMAGE_ABI}" \
        --device "$AVD_DEVICE" >/dev/null
}

write_local_properties() {
    local file content
    file="$(local_properties_file)"
    content="sdk.dir=${ANDROID_HOME}"
    if [ -f "$file" ] && [ "$(cat "$file")" = "$content" ]; then
        return
    fi
    printf '%s\n' "$content" > "$file"
    echo "wrote ${file} (gitignored)"
}

print_export_block() {
    cat <<EOF

Add these exports to your shell profile if they are not already present:

export ANDROID_HOME="${ANDROID_HOME}"
export ANDROID_SDK="\$ANDROID_HOME"
export ANDROID_SDK_ROOT="\$ANDROID_HOME"
export PATH="\$ANDROID_HOME/cmdline-tools/${CMDTOOLS_BUILD}/bin:\$ANDROID_HOME/emulator:\$ANDROID_HOME/platform-tools:\$PATH"
EOF
}

CHECK_FAILURES=0
check_ok() { printf '  ok    %s\n' "$1"; }
check_bad() {
    printf '  FAIL  %s\n' "$1"
    if [ "${2:-}" != "" ]; then
        printf '        next: %s\n' "$2"
    fi
    CHECK_FAILURES=$((CHECK_FAILURES + 1))
}

check_environment() {
    echo "checking headless Android SDK at ${ANDROID_HOME}"

    local major
    if major="$(jdk_major)" && [ "$major" = "$REQUIRED_JDK_MAJOR" ]; then
        check_ok "javac $(javac -version 2>&1 | awk '{print $2}')"
    else
        check_bad "JDK ${REQUIRED_JDK_MAJOR} not found" \
            "install a JDK ${REQUIRED_JDK_MAJOR} and ensure javac is on PATH"
    fi

    if [ -x "$(sdkmanager_bin)" ]; then
        local rev
        rev="$(cmdline_tools_revision "${ANDROID_HOME}/cmdline-tools/${CMDTOOLS_BUILD}")"
        if [ "$rev" = "$CMDTOOLS_REVISION" ]; then
            check_ok "sdkmanager ($(sdkmanager_bin), ${rev})"
        else
            check_bad \
                "command-line tools revision ${rev:-unknown}, expected ${CMDTOOLS_REVISION}" \
                "reconcile ${ANDROID_HOME}/cmdline-tools/${CMDTOOLS_BUILD} to build ${CMDTOOLS_BUILD}"
        fi
    else
        check_bad "sdkmanager missing" "run scripts/setup-android-sdk.sh"
    fi
    if [ -x "$(avdmanager_bin)" ]; then
        check_ok "avdmanager ($(avdmanager_bin))"
    else
        check_bad "avdmanager missing" "run scripts/setup-android-sdk.sh"
    fi
    if [ -x "$(adb_bin)" ]; then
        check_ok "adb ($(adb_bin))"
    else
        check_bad "platform-tools missing" "run scripts/setup-android-sdk.sh"
    fi
    if [ -f "${ANDROID_HOME}/platforms/${PLATFORM_ID}/android.jar" ]; then
        check_ok "platforms;${PLATFORM_ID}"
    else
        check_bad "platforms;${PLATFORM_ID} missing" "run scripts/setup-android-sdk.sh"
    fi
    if dir_nonempty "${ANDROID_HOME}/build-tools/${BUILD_TOOLS_VERSION}"; then
        check_ok "build-tools;${BUILD_TOOLS_VERSION}"
    else
        check_bad "build-tools;${BUILD_TOOLS_VERSION} missing" "run scripts/setup-android-sdk.sh"
    fi
    if [ -x "$(emulator_bin)" ]; then
        check_ok "emulator ($(emulator_bin))"
    else
        check_bad "emulator missing" "run scripts/setup-android-sdk.sh"
    fi
    if dir_nonempty "${ANDROID_HOME}/system-images/${PLATFORM_ID}/google_apis/${SYS_IMAGE_ABI}"; then
        check_ok "system-images;${PLATFORM_ID};google_apis;${SYS_IMAGE_ABI}"
    else
        check_bad "system image ${PLATFORM_ID}/${SYS_IMAGE_ABI} missing" \
            "run scripts/setup-android-sdk.sh"
    fi

    if [ -x "$(sdkmanager_bin)" ]; then
        local rows want rev i
        rows="$(installed_package_rows)"
        local ids=(
            "platform-tools"
            "platforms;${PLATFORM_ID}"
            "build-tools;${BUILD_TOOLS_VERSION}"
            "emulator"
            "system-images;${PLATFORM_ID};google_apis;${SYS_IMAGE_ABI}"
        )
        local pins=(
            "$PLATFORM_TOOLS_REVISION"
            "$PLATFORM_REVISION"
            "$BUILD_TOOLS_REVISION"
            "$EMULATOR_REVISION"
            "$SYS_IMAGE_REVISION"
        )
        for i in "${!ids[@]}"; do
            want="${pins[$i]}"
            rev="$(package_revision "$rows" "${ids[$i]}")"
            if [ "$rev" = "$want" ]; then
                check_ok "${ids[$i]} @ ${want}"
            elif [ -z "$rev" ]; then
                check_bad "${ids[$i]} not installed" "run scripts/setup-android-sdk.sh"
            else
                check_bad "${ids[$i]} is ${rev}, expected ${want}" \
                    "reconcile the package revision explicitly; setup never auto-upgrades"
            fi
        done
    fi

    local home ini
    home="$(avd_home)"
    ini="${home}/${AVD_NAME}.ini"
    if [ -f "$ini" ] && dir_nonempty "${home}/${AVD_NAME}.avd"; then
        check_ok "AVD ${AVD_NAME}"
    else
        check_bad "AVD ${AVD_NAME} missing" "run scripts/setup-android-sdk.sh"
    fi
    if avd_matches_pins; then
        check_ok "AVD ${AVD_NAME} pins match"
    else
        check_bad "AVD ${AVD_NAME} config.ini does not match the pinned image/device" \
            "inspect $(avd_home)/${AVD_NAME}.avd/config.ini"
    fi
    if avd_exists; then
        check_ok "emulator sees AVD ${AVD_NAME}"
    else
        check_bad "emulator -list-avds does not list ${AVD_NAME}" \
            "check ANDROID_AVD_HOME and re-run scripts/setup-android-sdk.sh"
    fi

    if [ -x "$(emulator_bin)" ]; then
        local accel_out
        if accel_out="$(run_with_deadline 30 "$(emulator_bin)" -accel-check 2>&1)"; then
            case "$accel_out" in
                *"accel: OK"* | *"Hypervisor"* | *"KVM"* | *"kvm"* | *"WHPX"* | *"HAXM"* | *"AEHD"*)
                    check_ok "emulator acceleration available" ;;
                *)
                    check_bad "emulator -accel-check found no usable hypervisor" \
                        "verify hardware virtualization is enabled for this host"
                    ;;
            esac
        else
            check_bad "emulator -accel-check failed" \
                "verify hardware virtualization (kern.hv_support on macOS, /dev/kvm on Linux)"
        fi
        if [ "$HOST_OS" = "Darwin" ] && [ "$(sysctl -n kern.hv_support 2>/dev/null || echo 0)" != "1" ]; then
            check_bad "kern.hv_support is not 1" "this Mac cannot hardware-accelerate the emulator"
        elif [ "$HOST_OS" = "Linux" ] && [ ! -w /dev/kvm ]; then
            check_bad "/dev/kvm is not writable" "enable KVM or add this user to the kvm group"
        fi
    fi

    local studio_found=""
    if [ "$HOST_OS" = "Darwin" ]; then
        for studio_dir in "/Applications/Android Studio.app" "$HOME/Applications/Android Studio.app"; do
            [ -d "$studio_dir" ] && studio_found="$studio_dir"
        done
    fi
    if [ -n "$studio_found" ]; then
        check_bad "Android Studio found at ${studio_found}" \
            "the headless SDK path must not share tools with Studio; uninstall it or use a different machine state"
    else
        check_ok "Android Studio absent"
    fi

    if [ -f "$(local_properties_file)" ] && \
       [ "$(cat "$(local_properties_file)")" = "sdk.dir=${ANDROID_HOME}" ]; then
        check_ok "drivers/android/local.properties"
    else
        check_bad "drivers/android/local.properties missing or stale" \
            "run scripts/setup-android-sdk.sh"
    fi

    echo
    if [ "$CHECK_FAILURES" -gt 0 ]; then
        echo "check failed: ${CHECK_FAILURES} problem(s)" >&2
        exit 1
    fi
    echo "all checks passed"
}

install_environment() {
    command -v python3 >/dev/null 2>&1 || fail "python3 is required" "install python3"
    detect_host
    require_jdk17
    mkdir -p "$ANDROID_HOME"
    install_cmdline_tools
    install_packages
    create_avd
    write_local_properties
    print_export_block
    echo
    echo "headless Android SDK setup complete at ${ANDROID_HOME}"
}

MODE="install"
main() {
case "${1:-}" in
    "") MODE="install" ;;
    --check) MODE="check" ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        fail "unknown argument: $1" "usage: $0 [--check]"
        ;;
esac

case "$MODE" in
    install) install_environment ;;
    check)
        command -v python3 >/dev/null 2>&1 || fail "python3 is required" "install python3"
        detect_host
        check_environment
        ;;
esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
