#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
bundle="$project_root/.work/native-ui/Clash Native.app"
destination='/Applications/Clash Native.app'
source "$project_root/scripts/installation-common.sh"
native_refuse_live_application
native_preflight_paths "$bundle" "$destination" "$project_root/.work/install"
[[ $EUID == 0 ]] || { print -u2 'Run with sudo after building; this installs only Clash Native, and does not register its helper or change network settings.'; exit 1; }
[[ -d "$bundle" && ! -L "$bundle" ]] || { print -u2 'Build scripts/native-ui-build.sh first.'; exit 1; }
/usr/bin/codesign --verify --deep --strict "$bundle"
if [[ -e "$destination" || -L "$destination" ]]; then
  if /bin/launchctl print system/io.github.jackieismpc.clash-native.helper >/dev/null 2>&1; then
    print -u2 'Unregister the native helper in Settings → Network before replacing the signed installation.'; exit 1
  fi
  [[ -d "$destination" && ! -L "$destination" && "$(/usr/bin/stat -f %u "$destination")" == 0 ]] || { print -u2 'Existing installation is not a root-owned directory; move it aside manually first.'; exit 1; }
fi
install_root="$project_root/.work/install"
mkdir -p "$install_root"
chmod 700 "$install_root"
stage="$(/usr/bin/mktemp -d "$install_root/stage.XXXXXXXX")"
trap 'rm -rf "$stage"' EXIT
/usr/bin/ditto "$bundle" "$stage/Clash Native.app"
/usr/sbin/chown -R root:wheel "$stage/Clash Native.app"
/bin/chmod -R go-w "$stage/Clash Native.app"
/usr/bin/codesign --verify --deep --strict "$stage/Clash Native.app"
if [[ -d "$destination" ]]; then
  backup="$(/usr/bin/mktemp -d "$install_root/previous.XXXXXXXX")"
  /bin/mv "$destination" "$backup/Clash Native.app"
  if ! /bin/mv "$stage/Clash Native.app" "$destination"; then
    /bin/mv "$backup/Clash Native.app" "$destination"
    exit 1
  fi
else
  /bin/mv "$stage/Clash Native.app" "$destination"
fi
/usr/bin/codesign --verify --deep --strict "$destination"
print -r -- "Installed $destination. Open it as your normal user; enable the privileged helper in Settings → Network before enabling system proxy, DNS or TUN."
