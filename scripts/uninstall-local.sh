#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
source "$project_root/scripts/installation-common.sh"
application='/Applications/Clash Native.app'
helper_id='io.github.jackieismpc.clash-native.helper'
application_id='io.github.jackieismpc.clash-verge-native'
[[ $EUID != 0 ]] || { print -u2 'Run as your normal user; the script requests sudo only for owned installation files.'; exit 1; }
native_refuse_live_application
[[ -d "$application" && ! -L "$application" ]] || { print -u2 'The fixed installation is missing; no files were removed.'; exit 1; }
user_paths=("$project_root/.work/native-ui" "$project_root/.work/integration" "$HOME/Library/Application Support/ClashNative" "$HOME/Library/Caches/$application_id" "$HOME/Library/Saved Application State/$application_id.savedState")
# Validate all user paths before unregistering or deleting anything.
native_preflight_paths "$application" "$project_root/.work/install" "${user_paths[@]}" "$HOME/Library/Preferences/$application_id.plist"
/usr/bin/codesign --verify --deep --strict "$application"
installed_data=$(/usr/bin/plutil -extract CvnDevelopmentDataDir raw -o - "$application/Contents/Info.plist")
[[ "$installed_data" == "$project_root/.work/native-ui/data" ]] || { print -u2 'Run the uninstall script from the source project that built this installation; no files were removed.'; exit 1; }
unregister_native() {
  "$application/Contents/MacOS/ClashNative" --prepare-uninstall
}
recover_native() {
  /usr/bin/sudo "$application/Contents/Helpers/clash-native-helper" --recover-for-uninstall
}
remove_native() {
  # Root recovery checks the daemon lock and exact engine identities first.
  /bin/rm -rf -- "${user_paths[@]}" || return $?
  if /usr/bin/defaults read "$application_id" >/dev/null 2>&1; then
    /usr/bin/defaults delete "$application_id" || return $?
  fi
  /bin/rm -f -- "$HOME/Library/Preferences/$application_id.plist" || return $?
  /usr/bin/sudo /bin/rm -rf -- "/var/db/$helper_id" "/var/run/$helper_id.sock" "$project_root/.work/install" || return $?
  # Keep recovery executables available until user-data cleanup also succeeds.
  /usr/bin/sudo /bin/rm -rf -- "$application" || return $?
}
native_uninstall_transaction unregister_native recover_native remove_native
print 'Removed Clash Native services, owned network settings, installed application, runtime data, logs and preferences. Source, development caches, toolchains and the daily Clash Verge were preserved.'
