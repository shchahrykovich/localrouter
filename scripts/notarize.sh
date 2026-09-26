#!/bin/bash
# Sign, notarize and staple a DMG.
#
#   scripts/notarize.sh <image.dmg>
#
# Credentials: environment, else .env.notarize (see scripts/notarize.env.example).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=release-lib.sh
. "$ROOT/scripts/release-lib.sh"

image="${1:-}"
[[ -f "$image" ]] || lr_die "usage: scripts/notarize.sh <image.dmg>"
lr_load_signing_env "$ROOT"
identity="$(lr_developer_id "${APPLE_TEAM_ID:-}")"
[[ -n "$identity" ]] || lr_die "no Developer ID Application identity in the keychain"
credentials=()
if lines="$(lr_notary_credentials)"; then
    while IFS= read -r line; do credentials+=("$line"); done <<<"$lines"
else
    lr_die "no notary credentials: copy scripts/notarize.env.example to .env.notarize and fill it in"
fi

lr_say "Signing the image"
codesign --force --sign "$identity" --timestamp "$image"
lr_say "Submitting to Apple (this takes a few minutes)"
xcrun notarytool submit "$image" "${credentials[@]}" --wait
lr_say "Stapling"
xcrun stapler staple "$image"
spctl --assess --type open --context context:primary-signature -v "$image"
lr_say "Notarized $image"
