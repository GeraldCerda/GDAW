#!/usr/bin/env bash
set -euo pipefail

kit_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [ "$(uname -s)" != "Darwin" ]; then
    printf 'Run this launcher on your Mac.\n' >&2
    exit 2
fi
binary="/Applications/GDAW Test.app/Contents/MacOS/GDAW-Test"
if [ ! -x "$binary" ]; then
    binary="$HOME/Applications/GDAW Test.app/Contents/MacOS/GDAW-Test"
fi
if [ ! -x "$binary" ]; then
    printf 'Build the tester first with: bash "%s/build_test.sh"\n' "$kit_dir" >&2
    exit 1
fi
export SOUNDCRAFT_CONFIG_DIR="$HOME/Library/Application Support/GDAW Test"
mkdir -p "$SOUNDCRAFT_CONFIG_DIR" "$kit_dir/logs"
launch_log="$kit_dir/logs/launch-$(date +%Y%m%d-%H%M%S).log"
printf 'Test settings: %s\nLaunch log: %s\n' "$SOUNDCRAFT_CONFIG_DIR" "$launch_log"
"$binary" "$@" 2>&1 | tee -a "$launch_log"
