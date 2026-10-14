#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
[[ $EUID != 0 ]] || { print -u2 'Run the developer app as your normal user; sudo is only for installation.'; exit 1; }
NATIVE_BUILD_PROFILE="${NATIVE_BUILD_PROFILE:-debug}" "$project_root/scripts/native-ui-build.sh"
/usr/bin/open "$project_root/.work/native-ui/Clash Native.app"
