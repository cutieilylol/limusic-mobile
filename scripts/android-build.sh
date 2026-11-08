#!/usr/bin/env bash
# Build the Android APK on Windows without Developer Mode.
#
# `tauri android build` compiles the Rust core and then wants to *symlink* the .so into jniLibs,
# which Windows refuses without Developer Mode. So: let it compile (and regenerate the Kotlin glue),
# copy the library in ourselves, and run Gradle with its own Rust step skipped.
#
#   scripts/android-build.sh            # release-profile Rust, fast settings (no LTO)
#   FULL=1 scripts/android-build.sh     # the real release profile (thin LTO, 1 codegen unit)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$PWD

(cd ui && pnpm build)

if [ -z "${FULL:-}" ]; then
	export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
fi
SO=target/aarch64-linux-android/release/libapp_lib.so
before=$(stat -c %Y "$SO" 2>/dev/null || echo 0)
(cd src-tauri && tauri android build --apk --target aarch64) || true
after=$(stat -c %Y "$SO" 2>/dev/null || echo 0)
if [ ! -f "$SO" ]; then echo "Rust build failed: no $SO" >&2; exit 1; fi
[ "$after" != "$before" ] || echo "note: $SO was already up to date"

python - <<'PY'
import json
base = json.load(open('src-tauri/tauri.conf.json', encoding='utf-8'))
over = json.load(open('src-tauri/tauri.android.conf.json', encoding='utf-8'))
def merge(a, b):
    for k, v in b.items():
        if isinstance(v, dict) and isinstance(a.get(k), dict): merge(a[k], v)
        else: a[k] = v
    return a
m = merge(base, over); m.pop('$schema', None)
json.dump(m, open('src-tauri/gen/android/app/src/main/assets/tauri.conf.json', 'w', encoding='utf-8'))
PY

JNI=src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a
mkdir -p "$JNI"
rm -f "$JNI/libapp_lib.so"
cp "$SO" "$JNI/libapp_lib.so"

(cd src-tauri/gen/android && ./gradlew.bat assembleArm64Release -x rustBuildArm64Release --console=plain)
APK=src-tauri/gen/android/app/build/outputs/apk/arm64/release/app-arm64-release.apk
ls -la "$APK"
