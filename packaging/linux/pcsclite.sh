#!/bin/sh
# Builds a static libpcsclite (the PC/SC client library) into <dir>, for linking the Linux binaries
# without a runtime dependency on the distribution's libpcsclite.
#
#   packaging/linux/pcsclite.sh <dir>     then: PCSC_LIB_DIR=<dir> PCSC_LIB_NAME=static=pcsclite cargo build
#
# Needs a C compiler, meson, ninja and flex. 2.4.1 is the oldest release whose client negotiates
# the protocol down to whatever pcscd it meets (any pcscd since 1.8.24, 2018); an older static
# client would only work with a pcscd of exactly its own protocol version.
set -eu
VERSION=2.5.3
SHA256=fe2bb733ecf95e52768907eb00da9f9f3daf180960283e514a05dcc090afa3bd

out=$(mkdir -p "$1" && cd "$1" && pwd)
# Build output goes to stderr; stdout carries only the resulting directory.
exec 3>&1 1>&2
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL -o "$work/pcsc-lite.tar.xz" "https://pcsclite.apdu.fr/files/pcsc-lite-$VERSION.tar.xz"
echo "$SHA256  $work/pcsc-lite.tar.xz" | sha256sum -c - > /dev/null
tar -xJf "$work/pcsc-lite.tar.xz" -C "$work"
cd "$work/pcsc-lite-$VERSION"
# Only the client library is used; the daemon is built without its optional hardware integrations.
meson setup build --buildtype=release --default-library=static \
    -Dlibsystemd=false -Dlibudev=false -Dlibusb=false -Dpolkit=false > /dev/null
ninja -C build libpcsclite.a > /dev/null
cp build/libpcsclite.a "$out/"
echo "$out" >&3
