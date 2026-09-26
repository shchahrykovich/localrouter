#!/bin/bash
# Build a release DMG: bump the version, build and sign the app, make the image.
#
#   scripts/release.sh [--patch|--minor|--major|--set X.Y.Z|--no-bump] [--notarize]
#
# --notarize signs with Developer ID and runs scripts/notarize.sh on the image.
# Without it the app is ad-hoc signed: fine for this Mac, not for other Macs.
# Output: dist/localrouter-<version>.dmg
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=release-lib.sh
. "$ROOT/scripts/release-lib.sh"

bump="patch"
set_to=""
notarize=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --patch|--minor|--major) bump="${1#--}"; shift ;;
        --set) bump="set"; set_to="$2"; shift 2 ;;
        --no-bump) bump="none"; shift ;;
        --notarize) notarize=1; shift ;;
        *) lr_die "unknown option $1" ;;
    esac
done

old="$(lr_version "$ROOT/Cargo.toml")"
IFS=. read -r major minor patch <<<"$old"
case "$bump" in
    patch) new="$major.$minor.$((patch + 1))" ;;
    minor) new="$major.$((minor + 1)).0" ;;
    major) new="$((major + 1)).0.0" ;;
    set) [[ "$set_to" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || lr_die "--set needs X.Y.Z"; new="$set_to" ;;
    none) new="$old" ;;
esac

restore_version() { [[ "$new" != "$old" ]] && lr_set_version "$ROOT/Cargo.toml" "$old" && (cd "$ROOT" && cargo metadata --format-version 1 >/dev/null 2>&1); }
if [[ "$new" != "$old" ]]; then
    lr_say "Version $old -> $new"
    lr_set_version "$ROOT/Cargo.toml" "$new"
    trap 'restore_version; lr_die "release failed; version restored to $old"' ERR
fi

sign_args=()
[[ $notarize == 1 ]] && sign_args=(--developer-id)
"$ROOT/scripts/build-app.sh" ${sign_args[@]+"${sign_args[@]}"}

mkdir -p "$ROOT/dist"
DMG="$ROOT/dist/localrouter-$new.dmg"
lr_say "Making $DMG"
staging="$(mktemp -d)"
ditto "$ROOT/build/LocalRouter.app" "$staging/LocalRouter.app"
ln -s /Applications "$staging/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "LocalRouter $new" -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$DMG"
rm -rf "$staging"

if [[ $notarize == 1 ]]; then
    "$ROOT/scripts/notarize.sh" "$DMG"
fi
trap - ERR
lr_say "Done: $DMG"
[[ "$new" != "$old" ]] && echo "Commit the version: git commit -am \"Release $new\""
exit 0
