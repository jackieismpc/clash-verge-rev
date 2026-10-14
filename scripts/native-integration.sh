#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
cd "$project_root"
scripts/native-core-assets.sh --offline
NATIVE_RUST_PACKAGE=clash-verge-native-helper scripts/native-core-build.sh build --offline
scripts/native-core-build.sh run --offline --bin integration -- --mihomo "$project_root/.work/engines/mihomo" "$@"
