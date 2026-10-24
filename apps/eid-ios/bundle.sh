#!/bin/sh
# Builds eID.app for an iPhone/iPad and signs it. Needs Xcode (the iOS SDK) and a signing identity.
#
#   BUNDLE_ID=rs.example.eid SIGN_IDENTITY="Apple Development: …" PROFILE=eid.mobileprovision ./bundle.sh
#
# Install the result with Xcode's Devices window or `xcrun devicectl device install app`.
set -eu
cd "$(dirname "$0")"
: "${BUNDLE_ID:?set BUNDLE_ID}" "${SIGN_IDENTITY:?set SIGN_IDENTITY}" "${PROFILE:?set PROFILE}"
TARGET=aarch64-apple-ios

cargo build --release --target "$TARGET" -p eid-ios
APP=../../target/$TARGET/release/eID.app
rm -rf "$APP" && mkdir -p "$APP"
cp "../../target/$TARGET/release/eid-ios" "$APP/"
sed "s/\$(BUNDLE_ID)/$BUNDLE_ID/" Info.plist > "$APP/Info.plist"
cp "$PROFILE" "$APP/embedded.mobileprovision"
codesign --force --sign "$SIGN_IDENTITY" --timestamp=none "$APP"
echo "$APP"
