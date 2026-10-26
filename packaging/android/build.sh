#!/bin/sh
# Builds the Android APK in Docker; prints its path. The first run builds the image (several GB).
#
#   packaging/android/build.sh
#
# Signs with your key when CARGO_APK_RELEASE_KEYSTORE (a path inside the repository) and
# CARGO_APK_RELEASE_KEYSTORE_PASSWORD are set; otherwise with a debug key kept in target/apk, good
# for `adb install` but not for Google Play.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
docker build --platform linux/amd64 -t eid-android-build "$root/packaging/android"

keystore=${CARGO_APK_RELEASE_KEYSTORE:-target/apk/debug.keystore}
password=${CARGO_APK_RELEASE_KEYSTORE_PASSWORD:-android}
# Cargo's registry and the build output live in a volume and target/apk, so rebuilds are incremental
# and never mix with the host's own target/ artifacts.
docker run --rm --platform linux/amd64 \
    -v "$root":/src \
    -v eid-cargo-registry:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/src/target/apk \
    -e CARGO_APK_RELEASE_KEYSTORE="/src/$keystore" \
    -e CARGO_APK_RELEASE_KEYSTORE_PASSWORD="$password" \
    eid-android-build \
    sh -c '
        set -eu
        if [ ! -f "$CARGO_APK_RELEASE_KEYSTORE" ]; then
            mkdir -p "$(dirname "$CARGO_APK_RELEASE_KEYSTORE")"
            keytool -genkeypair -keystore "$CARGO_APK_RELEASE_KEYSTORE" -storepass "$CARGO_APK_RELEASE_KEYSTORE_PASSWORD" \
                -alias androiddebugkey -keypass "$CARGO_APK_RELEASE_KEYSTORE_PASSWORD" -keyalg RSA -keysize 2048 \
                -validity 10000 -dname "CN=Android Debug,O=Android,C=US" > /dev/null
        fi
        cargo apk build --release -p eid-android
    '
ls "$root"/target/apk/release/apk/*.apk
