#!/bin/bash
# Build and install a LocalRouter instance on this Mac for development.
#
#   scripts/install.sh [--user] [--launch] [--suffix <suffix>]
#
# By default this installs LocalRouter-dev.app (suffix -dev), which runs next
# to the release with its own daemon, CLI, folders and ports 7080 and 7443
# (ADR 04). It never touches LocalRouter.app unless you pass --suffix "" on
# purpose.
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
suffix="-dev"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --user) dest_dir="$HOME/Applications"; shift ;;
        --launch) launch=1; shift ;;
        --suffix) [[ $# -ge 2 ]] || lr_die "--suffix needs a value, for example -dev, or \"\" for the release"
                  suffix="$2"; shift 2 ;;
        *) lr_die "unknown option $1" ;;
    esac
done
lr_check_suffix "$suffix"
app_name="$(lr_app_name "$suffix")"

"$ROOT/scripts/build-app.sh" --suffix "$suffix"
mkdir -p "$dest_dir"
dest="$dest_dir/$app_name.app"
# Only this instance: its bundle id, its app, its daemon.
osascript -e "tell application id \"$(lr_bundle_id "$suffix")\" to quit" >/dev/null 2>&1 || true
rm -rf "$dest"
ditto "$ROOT/build/$app_name.app" "$dest"
launchctl kickstart -k "gui/$(id -u)/$(lr_daemon_label "$suffix")" >/dev/null 2>&1 || true
lr_say "Installed $dest"
[[ $launch == 1 ]] && open "$dest"
exit 0
