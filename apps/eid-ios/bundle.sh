#!/bin/sh
# Builds eID.app for iPhone/iPad and packages it as eID.ipa; prints the .ipa's path. Needs Xcode.
#
#   apps/eid-ios/bundle.sh
#
# Signed when SIGN_IDENTITY ("Apple Development: …") and PROFILE (a .mobileprovision for BUNDLE_ID)
# are set; otherwise the .ipa is unsigned and must be signed before it installs.
set -eu
cd "$(dirname "$0")"
BUNDLE_ID=${BUNDLE_ID:-rs.eid.viewer}
TARGET=aarch64-apple-ios
TARGET_DIR=${CARGO_TARGET_DIR:-../../target}

cargo build --release --target "$TARGET" -p eid-ios
out="$TARGET_DIR/$TARGET/release"
rm -rf "$out/Payload" && mkdir -p "$out/Payload/eID.app"
app="$out/Payload/eID.app"
cp "$out/eid-ios" "$app/"
sed "s/\$(BUNDLE_ID)/$BUNDLE_ID/" Info.plist > "$app/Info.plist"
plutil -lint "$app/Info.plist" > /dev/null

if [ -n "${SIGN_IDENTITY:-}" ] && [ -n "${PROFILE:-}" ]; then
    cp "$PROFILE" "$app/embedded.mobileprovision"
    codesign --force --sign "$SIGN_IDENTITY" --timestamp=none "$app"
    ipa="$out/eID.ipa"
else
    ipa="$out/eID-unsigned.ipa"
fi
rm -f "$ipa"
(cd "$out" && zip -qr "$(basename "$ipa")" Payload)
echo "$ipa"
