#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
task_work="$project_root/.work/native-ui"
mkdir -p "$task_work/tmp" "$task_work/cache" "$task_work/config" "$task_work/security"
export TMPDIR="$task_work/tmp"
export CLANG_MODULE_CACHE_PATH="$task_work/cache/clang"
export SWIFT_MODULECACHE_PATH="$task_work/cache/swift"
build_profile="${NATIVE_BUILD_PROFILE:-debug}"
[[ "$build_profile" == release || "$build_profile" == debug ]] || { print -u2 'NATIVE_BUILD_PROFILE must be release or debug.'; exit 1; }
rust_profile=()
[[ "$build_profile" == release ]] && rust_profile+=(--release)
"$project_root/scripts/native-core-build.sh" build --offline --lib "${rust_profile[@]}"
NATIVE_RUST_PACKAGE=clash-verge-native-helper "$project_root/scripts/native-core-build.sh" build --offline "${rust_profile[@]}"
"$project_root/scripts/native-core-assets.sh" --offline
/usr/bin/clang -dynamiclib -arch arm64 -mmacosx-version-min=26.0 -I "$project_root/crates/clash-verge-native-core/include" "$project_root/crates/clash-verge-native-core/tests/fixtures/abi1.c" -o "$task_work/abi1-fixture.dylib"
/usr/bin/clang -dynamiclib -arch arm64 -mmacosx-version-min=26.0 -I "$project_root/crates/clash-verge-native-core/include" "$project_root/crates/clash-verge-native-core/tests/fixtures/no_state_events.c" -o "$task_work/no-state-fixture.dylib"
export CVN_NO_STATE_LIBRARY_PATH="$task_work/no-state-fixture.dylib"
export CVN_REAL_LIBRARY_PATH="$project_root/.work/target/aarch64-apple-darwin/$build_profile/libclash_verge_native_core.dylib"
export CVN_ABI1_LIBRARY_PATH="$task_work/abi1-fixture.dylib"
export CVN_LIBRARY_PATH="$project_root/.work/target/aarch64-apple-darwin/$build_profile/libclash_verge_native_core.dylib"
export CVN_HOST_PATH="$project_root/.work/target/aarch64-apple-darwin/$build_profile/clash-native-helper"
export CVN_MIHOMO_PATH="$project_root/.work/engines/mihomo"
export CVN_TEST_ROOT="$project_root/.work/integration"
mkdir -p "$CVN_TEST_ROOT"
developer_path="${DEVELOPER_DIR:-$(/usr/bin/xcode-select -p)}"
if [[ -d "$developer_path/Platforms/MacOSX.platform/Developer/Library/Frameworks" ]]; then
  testing_frameworks="$developer_path/Platforms/MacOSX.platform/Developer/Library/Frameworks"
  testing_plugin="$developer_path/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
else
  testing_frameworks="$developer_path/Library/Developer/Frameworks"
  testing_plugin="$developer_path/usr/lib/swift/host/plugins/testing/libTestingMacros.dylib"
fi
if [[ ! -d "$testing_frameworks/Testing.framework" || ! -f "$testing_plugin" ]]; then
  print -u2 -- "The installed Swift toolchain is missing Swift Testing support."
  exit 1
fi
/usr/bin/swift test --package-path "$project_root/native" \
  --configuration "$build_profile" --build-system native --scratch-path "$task_work/build-native" --cache-path "$task_work/cache" \
  --config-path "$task_work/config" --security-path "$task_work/security" \
  -Xswiftc -F -Xswiftc "$testing_frameworks" \
  -Xswiftc -load-plugin-library -Xswiftc "$testing_plugin" \
  -Xlinker -rpath -Xlinker "$testing_frameworks" "$@"
