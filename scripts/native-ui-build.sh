#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
task_work="$project_root/.work/native-ui"
mkdir -p "$task_work/tmp" "$task_work/cache" "$task_work/config" "$task_work/security"
export TMPDIR="$task_work/tmp"
export CLANG_MODULE_CACHE_PATH="$task_work/cache/clang"
export SWIFT_MODULECACHE_PATH="$task_work/cache/swift"
/usr/bin/swift build --package-path "$project_root/native" \
  --build-system native --scratch-path "$task_work/build-native" --cache-path "$task_work/cache" \
  --config-path "$task_work/config" --security-path "$task_work/security" "$@"
bundle="$task_work/Clash Native.app"
binary_dir="$(/usr/bin/swift build --package-path "$project_root/native" \
  --build-system native --scratch-path "$task_work/build-native" --cache-path "$task_work/cache" \
  --config-path "$task_work/config" --security-path "$task_work/security" \
  --show-bin-path "$@")"
mkdir -p "$bundle/Contents/MacOS"
cp "$binary_dir/ClashNative" "$bundle/Contents/MacOS/ClashNative"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>ClashNative</string>
<key>CFBundleIdentifier</key><string>dev.clash.native.preview</string>
<key>CFBundleName</key><string>Clash Native</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>26.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
/usr/bin/codesign --force --sign - "$bundle"
print -r -- "$bundle"
