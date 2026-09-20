#!/usr/bin/env bash
# Check a release bundle against what Google Play requires, before uploading.
#
#   android/scripts/preflight.sh [path/to/app.aab]      (default: android/dist/jsonquery-<version>.aab)
#
# Fails (non-zero) on anything Play would reject or that would ship broken:
#   - not a valid bundle
#   - targetSdkVersion below 36 (required for new uploads since 2026-08-31)
#   - no 64-bit native library, or a 64-bit library not aligned for 16 KB pages
#   - debuggable
#   - unsigned, or signed with the debug key
#   - the store listing under play/ violates Play's limits
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

version="$(workspace_version)"
aab="${1:-$ANDROID_DIR/dist/jsonquery-$version.aab}"
[ -f "$aab" ] || die "no bundle at $aab; build one with: android/scripts/build.sh aab"
case "$aab" in "$REPO_ROOT"/*) ;; *) die "the bundle must be inside the repo (it is mounted into the container)";; esac
aab_in_container="/work/${aab#"$REPO_ROOT"/}"

log "Checking ${aab#"$REPO_ROOT"/}"
report="$(mktemp)"
trap 'rm -f "$report"' EXIT
set +e
in_container -- bash -s -- "$aab_in_container" 2>&1 <<'INNER' | tee "$report"
set -uo pipefail
aab="$1"
fail=0
ok()   { echo "  ok    $*"; }
bad()  { echo "  FAIL  $*"; fail=1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# 1. A valid bundle at all.
if bundletool validate --bundle "$aab" >"$work/validate.txt" 2>&1; then ok "bundletool validate"; else bad "bundletool validate"; cat "$work/validate.txt"; fi

# 2. Manifest facts, read from the bundle's base module.
bundletool dump manifest --bundle "$aab" >"$work/manifest.xml" 2>/dev/null
attr() { grep -o "$1=\"[^\"]*\"" "$work/manifest.xml" | head -1 | cut -d'"' -f2; }
package="$(attr package)"; target="$(grep -o 'targetSdkVersion[^/]*' "$work/manifest.xml" | grep -o '[0-9]\+' | head -1)"
vcode="$(grep -o 'versionCode[^ ]*' "$work/manifest.xml" | grep -o '[0-9]\+' | head -1)"
echo "  package $package, versionCode $vcode, targetSdk $target"
[ "${target:-0}" -ge 36 ] && ok "targetSdkVersion >= 36" || bad "targetSdkVersion ${target:-?} < 36 (Play requires 36 for new uploads)"
grep -q 'android:debuggable="true"' "$work/manifest.xml" && bad "the manifest says debuggable" || ok "not debuggable"

# 3. Native libraries: 64-bit present; every 64-bit one aligned for 16 KB pages.
unzip -q -o "$aab" 'base/lib/*' -d "$work/bundle" 2>/dev/null
readelf="$(ls /opt/android-sdk/ndk/*/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf | head -1)"
libs="$(find "$work/bundle" -name '*.so' 2>/dev/null | sort)"
[ -n "$libs" ] || bad "no native libraries in the bundle"
echo "$libs" | grep -q '/arm64-v8a/' && ok "arm64-v8a present" || bad "no arm64-v8a library (Play requires 64-bit support)"
for lib in $libs; do
    abi="$(basename "$(dirname "$lib")")"
    case "$abi" in arm64-v8a|x86_64) ;; *) continue ;; esac
    # Every LOAD segment's alignment must be a multiple of 0x4000 (16 KB).
    aligns="$("$readelf" -lW "$lib" | awk '$1=="LOAD" {print $NF}' | sort -u)"
    aligned=1
    for a in $aligns; do (( a % 16384 == 0 )) || aligned=0; done
    if [ -n "$aligns" ] && [ "$aligned" = 1 ]; then
        ok "$abi/$(basename "$lib") aligned for 16 KB pages (LOAD align: $(echo $aligns))"
    else
        bad "$abi/$(basename "$lib") is NOT 16 KB aligned (LOAD align: $(echo $aligns))"
    fi
done
# Nothing but our own library may ship (build tooling has leaked host .so files into the bundle before).
stray="$(echo "$libs" | grep -v '/libjsonquery_android\.so$' || true)"
[ -z "$stray" ] && ok "only libjsonquery_android.so is packaged" || bad "unexpected native libraries: $(echo $stray | sed "s#$work/bundle/##g")"

# 4. Signature: present, and not the debug key.
if jarsigner -verify "$aab" >"$work/sig.txt" 2>&1 && grep -q 'jar verified' "$work/sig.txt"; then
    signer="$(keytool -printcert -jarfile "$aab" 2>/dev/null | grep -m1 'Owner:' | sed 's/^Owner: *//')"
    if echo "$signer" | grep -q 'Android Debug'; then bad "signed with the Android debug key"; else ok "signed by: ${signer:-unknown}"; fi
else
    bad "not signed (configure the upload key: see android/README.md, Signing)"
fi

# 5. Size.
size="$(stat -c %s "$aab")"; echo "  bundle size: $((size / 1024 / 1024)) MiB"
[ "$size" -lt $((200 * 1024 * 1024)) ] && ok "under Play's 200 MiB base-module limit" || bad "bundle over 200 MiB"

# 6. The store listing.
if python3 /work/android/scripts/play_publish.py --check-listing-only; then ok "store listing within Play limits"; else bad "store listing problems (above)"; fi

echo
[ "$fail" -eq 0 ] && echo "preflight passed" || echo "preflight FAILED"
exit "$fail"
INNER
status=${PIPESTATUS[0]}
set -e
# A gate that ran nothing must not pass: insist on the verdict line itself.
grep -q '^preflight passed$' "$report" || {
    grep -q '^preflight FAILED$' "$report" || log "preflight did not run to completion (no verdict printed)"
    exit 1
}
exit "$status"
