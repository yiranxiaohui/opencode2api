#!/bin/sh
set -eu

profile_dir=""
for arg in "$@"; do
    case "$arg" in
        --user-data-dir=*) profile_dir=${arg#--user-data-dir=} ;;
    esac
done

if [ -z "$profile_dir" ]; then
    echo "missing --user-data-dir" >&2
    exit 2
fi

browser_home=/tmp/opencode2api-browser-home
mkdir -p "$profile_dir" "$browser_home"
chown -R nobody:nogroup "$profile_dir" "$browser_home"
exec gosu nobody:nogroup env HOME="$browser_home" /usr/bin/chromium "$@"
