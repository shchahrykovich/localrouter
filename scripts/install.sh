#!/bin/bash
# Build and install LocalRouter.app on this Mac for development.
#
#   scripts/install.sh [--user] [--launch]
#
# --user installs to ~/Applications instead of /Applications.
# The build is ad-hoc signed, so the updater will not replace it; install a
# release from GitHub to get self-updates.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=release-lib.sh
. "$ROOT/scripts/release-lib.sh"

dest_dir="/Applications"
launch=0
for arg in "$@"; do
    case "$arg" in
        --user) dest_dir="$HOME/Applications" ;;
        --launch) launch=1 ;;
        *) lr_die "unknown option $arg" ;;
    esac
done

"$ROOT/scripts/build-app.sh"
mkdir -p "$dest_dir"
dest="$dest_dir/LocalRouter.app"
osascript -e 'tell application id "dev.localrouter.app" to quit' >/dev/null 2>&1 || true
rm -rf "$dest"
ditto "$ROOT/build/LocalRouter.app" "$dest"
launchctl kickstart -k "gui/$(id -u)/$LR_DAEMON_LABEL" >/dev/null 2>&1 || true
lr_say "Installed $dest"
[[ $launch == 1 ]] && open "$dest"
exit 0
