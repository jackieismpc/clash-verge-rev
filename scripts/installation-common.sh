# Shared installation transaction; sourced by scripts and isolated fixture tests.
native_preflight_paths() {
  local candidate_path check_ancestor
  for candidate_path in "$@"; do
    [[ "$candidate_path" == /* && "$candidate_path" != / ]] || return 1
    check_ancestor="$candidate_path"
    while [[ "$check_ancestor" != / ]]; do
      [[ ! -L "$check_ancestor" ]] || { print -u2 -- "Refusing redirected installation data: $check_ancestor"; return 1; }
      check_ancestor="${check_ancestor:h}"
    done
  done
}
native_uninstall_transaction() {
  local unregister="$1" recover="$2" remove="$3"
  "$unregister" || return $?
  "$recover" || return $?
  "$remove"
}
native_refuse_live_application() {
  if /bin/ps -axo comm= | /usr/bin/awk '$0 ~ /\/ClashNative$/ {found=1} END {exit !found}'; then
    print -u2 'Quit all Clash Native instances before changing the installation.'
    return 1
  fi
}
