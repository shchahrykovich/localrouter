#!/bin/bash
# Remove build artifacts. publish.sh runs it after a release is uploaded.
#
#   scripts/clean.sh [--dry-run] [--keep-dmg <file>]
#
# Removes:
#   build/                 the assembled apps (install.sh has copied its app away)
#   dist/*.dmg             release images; --keep-dmg keeps one (the others are on GitHub)
#   target/                the Rust build cache (cargo clean), also the debug builds of tests
#   apps/menubar/.build/   the Swift build cache
#
# All of it is made again by the next build, which is then slower.
# --dry-run prints what would go and its size, and removes nothing.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=release-lib.sh
. "$ROOT/scripts/release-lib.sh"

dry=0
keep=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) dry=1; shift ;;
        --keep-dmg) [[ $# -ge 2 ]] || lr_die "--keep-dmg needs a file"; keep="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"; shift 2 ;;
        *) lr_die "unknown option $1" ;;
    esac
done

targets=()
for dir in "$ROOT/build" "$ROOT/target" "$ROOT/apps/menubar/.build"; do
    [[ -e "$dir" ]] && targets+=("$dir")
done
if [[ -d "$ROOT/dist" ]]; then
    while IFS= read -r -d '' dmg; do
        [[ "$dmg" == "$keep" ]] || targets+=("$dmg")
    done < <(find "$ROOT/dist" -maxdepth 1 -name 'localrouter-*.dmg' -print0)
fi

if [[ ${#targets[@]} -eq 0 ]]; then
    lr_say "Nothing to clean"
    exit 0
fi

total_kb=0
for t in "${targets[@]}"; do
    kb="$(du -sk "$t" 2>/dev/null | awk '{print $1}')"
    total_kb=$((total_kb + ${kb:-0}))
    printf '    %8s MB  %s\n' "$(( ${kb:-0} / 1024 ))" "${t#"$ROOT"/}"
done

if [[ $dry == 1 ]]; then
    lr_say "Would free $((total_kb / 1024)) MB (dry run, nothing removed)"
    exit 0
fi
for t in "${targets[@]}"; do
    rm -rf "$t"
done
lr_say "Freed $((total_kb / 1024)) MB"
