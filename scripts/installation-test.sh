#!/bin/zsh
set -euo pipefail
project_root="${0:A:h:h}"
source "$project_root/scripts/installation-common.sh"
mkdir -p "$project_root/.work/installation-tests"
fixture=$(/usr/bin/mktemp -d "$project_root/.work/installation-tests/run.XXXXXXXX")
trap '/bin/rm -rf -- "$fixture"' EXIT
mkdir "$fixture/app" "$fixture/data" "$fixture/outside"
print retained > "$fixture/data/journal"
print preserved > "$fixture/outside/sentinel"
ln -s "$fixture/outside" "$fixture/redirected"
if native_preflight_paths "$fixture/redirected/cache" 2>/dev/null; then exit 1; fi
native_preflight_paths "$fixture/app" "$fixture/data"
trace=()
fixture_unregister() { trace+=(unregister); return 0; }
fixture_failed_unregister() { trace+=(unregister); return 1; }
fixture_failed_recovery() { trace+=(recover); return 1; }
fixture_recover() { trace+=(recover); /bin/rm -- "$fixture/data/journal"; }
fixture_remove() { trace+=(remove); /bin/rm -rf -- "$fixture/app" "$fixture/data"; }
if native_uninstall_transaction fixture_failed_unregister fixture_recover fixture_remove; then exit 1; fi
[[ "${(j:,:)trace}" == unregister && -f "$fixture/data/journal" && -d "$fixture/app" ]]
trace=()
if native_uninstall_transaction fixture_unregister fixture_failed_recovery fixture_remove; then exit 1; fi
[[ "${(j:,:)trace}" == unregister,recover && -f "$fixture/data/journal" && -d "$fixture/app" ]]
trace=()
native_uninstall_transaction fixture_unregister fixture_recover fixture_remove
[[ "${(j:,:)trace}" == unregister,recover,remove && ! -e "$fixture/app" && ! -e "$fixture/data" && -f "$fixture/outside/sentinel" ]]
print 'Installation fixtures passed: redirected paths rejected, failed unregister/recovery preserved files, successful recovery preceded scoped deletion.'
