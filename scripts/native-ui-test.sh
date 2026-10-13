#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
task_work="$project_root/.work/native-ui"
mkdir -p "$task_work/tmp" "$task_work/cache" "$task_work/config" "$task_work/security"
export TMPDIR="$task_work/tmp"
export CLANG_MODULE_CACHE_PATH="$task_work/cache/clang"
export SWIFT_MODULECACHE_PATH="$task_work/cache/swift"
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
  --build-system native --scratch-path "$task_work/build-native" --cache-path "$task_work/cache" \
  --config-path "$task_work/config" --security-path "$task_work/security" \
  -Xswiftc -F -Xswiftc "$testing_frameworks" \
  -Xswiftc -load-plugin-library -Xswiftc "$testing_plugin" \
  -Xlinker -rpath -Xlinker "$testing_frameworks" "$@"
