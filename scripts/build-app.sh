#!/bin/bash
# Build LocalRouter.app: Rust daemon and CLI, Swift menu bar app, plists, icon.
#
#   scripts/build-app.sh [--suffix <suffix>] [--sign <identity>] [--developer-id] [--out <dir>]
#
# --suffix builds another instance (ADR 04): --suffix -dev makes
# LocalRouter-dev.app with its own bundle id, daemon, CLI, folders and ports,
# so it runs next to the release. The default is the release (no suffix).
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
suffix=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --suffix) suffix="$2"; shift 2 ;;
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

lr_check_suffix "$suffix"
app_name="$(lr_app_name "$suffix")"
bundle_id="$(lr_bundle_id "$suffix")"
daemon_label="$(lr_daemon_label "$suffix")"
# The daemon and the CLI read their instance from their own file names.
daemon_program="localrouterd$suffix"
cli_program="localrouter$suffix"

version="$(lr_version "$ROOT/Cargo.toml")"
[[ -n "$version" ]] || lr_die "cannot read the version from Cargo.toml"
APP="$out/$app_name.app"

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
cp "$ROOT/target/release/localrouterd" "$APP/Contents/MacOS/$daemon_program"
cp "$ROOT/target/release/localrouter" "$APP/Contents/Helpers/$cli_program"
for program in MacOS/LocalRouter "MacOS/$daemon_program" "Helpers/$cli_program"; do
    [[ -x "$APP/Contents/$program" ]] || lr_die "missing $program in the bundle"
done
cmp -s "$swift_bin" "$APP/Contents/MacOS/LocalRouter" || lr_die "Contents/MacOS/LocalRouter is not the Swift app"
render() {
    sed -e "s|@VERSION@|$version|g" -e "s|@BUNDLE_ID@|$bundle_id|g" \
        -e "s|@DAEMON_LABEL@|$daemon_label|g" -e "s|@MIN_MACOS@|$LR_MIN_MACOS|g" \
        -e "s|@APP_NAME@|$app_name|g" -e "s|@SUFFIX@|$suffix|g" -e "s|@DAEMON_PROGRAM@|$daemon_program|g" "$1"
}
render "$ROOT/scripts/Info.plist.in" > "$APP/Contents/Info.plist"
render "$ROOT/scripts/daemon.plist.in" > "$APP/Contents/Library/LaunchAgents/$daemon_label.plist"
plutil -lint -s "$APP/Contents/Info.plist" "$APP/Contents/Library/LaunchAgents/$daemon_label.plist"

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
# The note that Install Claude Code Instructions… links into ~/.claude. The
# bundled CLI prints it, filled in for this instance (ADR 04).
# The CLI writes to a temp file first: writing from it straight into the new
# bundle fails with "Input/output error" when build-app.sh runs it.
note="$(mktemp)"
"$APP/Contents/Helpers/$cli_program" note > "$note"
# The CLI names the instance it read from its own file name: it must be this one.
[[ "$(head -1 "$note")" == "# $app_name" ]] || lr_die "the bundled CLI does not think it is $app_name: $(head -1 "$note")"
mv "$note" "$APP/Contents/Resources/LocalRouter.md"
chmod 644 "$APP/Contents/Resources/LocalRouter.md"

lr_say "Signing ($([[ "$identity" == "-" ]] && echo ad-hoc || echo "Developer ID"))"
if [[ "$identity" == "-" ]]; then
    flags=(--force --sign - --timestamp=none)
else
    flags=(--force --options runtime --timestamp --sign "$identity")
fi
# Nested programs first, then the bundle.
codesign "${flags[@]}" --identifier "$bundle_id.daemon" "$APP/Contents/MacOS/$daemon_program"
codesign "${flags[@]}" --identifier "$bundle_id.cli" "$APP/Contents/Helpers/$cli_program"
codesign "${flags[@]}" "$APP"
codesign --verify --deep --strict "$APP"
lr_say "Built $APP ($version)"
