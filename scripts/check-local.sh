#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
cd "$project_root"
git diff --check
for package in clash-verge-native-core clash-verge-build-tools; do
  NATIVE_RUST_PACKAGE="$package" scripts/native-core-build.sh fmt -- --check
  NATIVE_RUST_PACKAGE="$package" scripts/native-core-build.sh clippy --offline --all-targets -- -D warnings
  NATIVE_RUST_PACKAGE="$package" scripts/native-core-build.sh test --offline
done
scripts/native-ui-test.sh
