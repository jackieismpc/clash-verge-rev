#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
task_work="$project_root/.work/native-ui"
source "$project_root/scripts/installation-common.sh"
native_refuse_live_application
mkdir -p "$task_work/tmp" "$task_work/cache" "$task_work/config" "$task_work/security"
export TMPDIR="$task_work/tmp"
export CLANG_MODULE_CACHE_PATH="$task_work/cache/clang"
export SWIFT_MODULECACHE_PATH="$task_work/cache/swift"
build_profile="${NATIVE_BUILD_PROFILE:-release}"
[[ "$build_profile" == release || "$build_profile" == debug ]] || { print -u2 'NATIVE_BUILD_PROFILE must be release or debug.'; exit 1; }
rust_profile=()
[[ "$build_profile" == release ]] && rust_profile+=(--release)
"$project_root/scripts/native-core-build.sh" build --offline --lib "${rust_profile[@]}"
NATIVE_RUST_PACKAGE=clash-verge-native-helper "$project_root/scripts/native-core-build.sh" build --offline "${rust_profile[@]}"
"$project_root/scripts/native-core-assets.sh" --offline
/usr/bin/swift build --package-path "$project_root/native" \
  --configuration "$build_profile" --build-system native --scratch-path "$task_work/build-native" --cache-path "$task_work/cache" \
  --config-path "$task_work/config" --security-path "$task_work/security" "$@"
bundle="$task_work/Clash Native.app"
binary_dir="$(/usr/bin/swift build --package-path "$project_root/native" \
  --configuration "$build_profile" --build-system native --scratch-path "$task_work/build-native" --cache-path "$task_work/cache" \
  --config-path "$task_work/config" --security-path "$task_work/security" \
  --show-bin-path "$@")"
native_refuse_live_application
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Frameworks" "$bundle/Contents/Resources" "$bundle/Contents/Helpers" "$bundle/Contents/Library/LaunchDaemons"
cp "$project_root/.work/target/aarch64-apple-darwin/$build_profile/libclash_verge_native_core.dylib" "$bundle/Contents/Frameworks/"
cp "$project_root/.work/engines/mihomo" "$bundle/Contents/Resources/"
cp "$project_root/native/Resources/AppIcon.icns" "$bundle/Contents/Resources/"
cp "$project_root/.work/target/aarch64-apple-darwin/$build_profile/clash-native-helper" "$bundle/Contents/Helpers/"
cp "$project_root/native/Resources/LaunchDaemons/io.github.jackieismpc.clash-native.helper.plist" "$bundle/Contents/Library/LaunchDaemons/"
/usr/bin/codesign --force --sign - "$bundle/Contents/Frameworks/libclash_verge_native_core.dylib"
/usr/bin/codesign --force --sign - "$bundle/Contents/Resources/mihomo"
/usr/bin/codesign --force --sign - "$bundle/Contents/Helpers/clash-native-helper"
cp "$binary_dir/ClashNative" "$bundle/Contents/MacOS/ClashNative"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>ClashNative</string>
<key>CFBundleIdentifier</key><string>io.github.jackieismpc.clash-verge-native</string>
<key>CFBundleName</key><string>Clash Native</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>26.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
/usr/bin/plutil -insert CvnDevelopmentDataDir -string "$task_work/data" "$bundle/Contents/Info.plist"
/usr/bin/codesign --force --sign - "$bundle"
/usr/bin/codesign --verify --deep --strict "$bundle"
print -r -- "$bundle"
