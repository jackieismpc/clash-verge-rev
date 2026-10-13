#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
cargo_flags=()
if (( ${@[(I)--offline]} )); then cargo_flags+=(--offline); fi
NATIVE_RUST_PACKAGE=clash-verge-build-tools "$project_root/scripts/native-core-build.sh" run --quiet "${cargo_flags[@]}" -- "$@"
