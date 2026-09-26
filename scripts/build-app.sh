#!/bin/bash
# Build LocalRouter.app: Rust daemon and CLI, Swift menu bar app, plists, icon.
#
#   scripts/build-app.sh [--sign <identity>] [--developer-id] [--out <dir>]
#
# Default signing is ad-hoc (runs on this Mac; other Macs show a Gatekeeper
# warning). --developer-id picks the "Developer ID Application" identity from
# the keychain and signs with the hardened runtime, as notarization requires.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=release-lib.sh
. "$ROOT/scripts/release-lib.sh"

identity="-"
out="$ROOT/build"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --sign) identity="$2"; shift 2 ;;
        --developer-id)
            lr_load_signing_env "$ROOT"
            identity="$(lr_developer_id "${APPLE_TEAM_ID:-}")"
            [[ -n "$identity" ]] || lr_die "no Developer ID Application identity in the keychain"
            shift ;;
        --out) out="$2"; shift 2 ;;
        *) lr_die "unknown option $1" ;;
    esac
done

version="$(lr_version "$ROOT/Cargo.toml")"
[[ -n "$version" ]] || lr_die "cannot read the version from Cargo.toml"
APP="$out/LocalRouter.app"

lr_say "Building Rust binaries $version"
(cd "$ROOT" && cargo build --release --quiet -p localrouterd -p localrouter)

lr_say "Building the Swift app"
swift build -c release --package-path "$ROOT/apps/menubar" --product LocalRouter >/dev/null
swift_bin="$(swift build -c release --package-path "$ROOT/apps/menubar" --show-bin-path)/LocalRouter"

lr_say "Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Resources" "$APP/Contents/Library/LaunchAgents"
cp "$swift_bin" "$APP/Contents/MacOS/LocalRouter"
# The CLI goes to Helpers: "localrouter" and "LocalRouter" are the same name
# on a case-insensitive file system, so it cannot sit next to the app in MacOS.
cp "$ROOT/target/release/localrouterd" "$APP/Contents/MacOS/"
cp "$ROOT/target/release/localrouter" "$APP/Contents/Helpers/"
for program in MacOS/LocalRouter MacOS/localrouterd Helpers/localrouter; do
    [[ -x "$APP/Contents/$program" ]] || lr_die "missing $program in the bundle"
done
cmp -s "$swift_bin" "$APP/Contents/MacOS/LocalRouter" || lr_die "Contents/MacOS/LocalRouter is not the Swift app"
render() {
    sed -e "s|@VERSION@|$version|g" -e "s|@BUNDLE_ID@|$LR_BUNDLE_ID|g" \
        -e "s|@DAEMON_LABEL@|$LR_DAEMON_LABEL|g" -e "s|@MIN_MACOS@|$LR_MIN_MACOS|g" "$1"
}
render "$ROOT/scripts/Info.plist.in" > "$APP/Contents/Info.plist"
render "$ROOT/scripts/daemon.plist.in" > "$APP/Contents/Library/LaunchAgents/$LR_DAEMON_LABEL.plist"
plutil -lint -s "$APP/Contents/Info.plist" "$APP/Contents/Library/LaunchAgents/$LR_DAEMON_LABEL.plist"

icon_cache="$ROOT/build/AppIcon.icns"
if [[ ! -f "$icon_cache" || "$ROOT/scripts/make-icon.swift" -nt "$icon_cache" ]]; then
    lr_say "Drawing the icon"
    tmp="$(mktemp -d)"
    swift "$ROOT/scripts/make-icon.swift" "$tmp/icon.png"
    mkdir "$tmp/AppIcon.iconset"
    for s in 16 32 128 256 512; do
        sips -z $s $s "$tmp/icon.png" --out "$tmp/AppIcon.iconset/icon_${s}x${s}.png" >/dev/null
        sips -z $((s * 2)) $((s * 2)) "$tmp/icon.png" --out "$tmp/AppIcon.iconset/icon_${s}x${s}@2x.png" >/dev/null
    done
    mkdir -p "$ROOT/build"
    iconutil -c icns "$tmp/AppIcon.iconset" -o "$icon_cache"
    rm -rf "$tmp"
fi
cp "$icon_cache" "$APP/Contents/Resources/AppIcon.icns"

lr_say "Signing ($([[ "$identity" == "-" ]] && echo ad-hoc || echo "Developer ID"))"
if [[ "$identity" == "-" ]]; then
    flags=(--force --sign - --timestamp=none)
else
    flags=(--force --options runtime --timestamp --sign "$identity")
fi
# Nested programs first, then the bundle.
codesign "${flags[@]}" --identifier "$LR_BUNDLE_ID.daemon" "$APP/Contents/MacOS/localrouterd"
codesign "${flags[@]}" --identifier "$LR_BUNDLE_ID.cli" "$APP/Contents/Helpers/localrouter"
codesign "${flags[@]}" "$APP"
codesign --verify --deep --strict "$APP"
lr_say "Built $APP ($version)"
