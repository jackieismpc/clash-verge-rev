#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
cd "$project_root"
scripts/native-core-assets.sh --offline
scripts/native-core-build.sh run --offline --bin integration -- --mihomo "$project_root/.work/engines/mihomo" "$@"
