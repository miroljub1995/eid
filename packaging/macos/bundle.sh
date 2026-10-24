#!/bin/sh
# Wraps the desktop binary into eID.app and prints its path.
#
#   packaging/macos/bundle.sh <eid-viewer binary> <output dir>
#
# The bundle is signed ad hoc (no Apple identity), so Gatekeeper asks once: right-click › Open.
set -eu
binary=$1
out=$2
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$(dirname "$0")/../../Cargo.toml" | head -1)

app="$out/eID.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$binary" "$app/Contents/MacOS/eid-viewer"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>sr-Latn</string>
	<key>CFBundleDisplayName</key>
	<string>eID</string>
	<key>CFBundleExecutable</key>
	<string>eid-viewer</string>
	<key>CFBundleIdentifier</key>
	<string>rs.eid.viewer</string>
	<key>CFBundleName</key>
	<string>eID</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>$version</string>
	<key>CFBundleVersion</key>
	<string>$version</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" > /dev/null
codesign --force --sign - "$app"
echo "$app"
