#!/bin/sh
# Builds the Linux release binaries in containers, for this machine's architecture, into
# target/linux-dist; prints that directory.
#
#   eid         the CLI: fully static (musl), runs on any Linux
#   eid-viewer  the app: glibc 2.35+; libpcsclite linked in, so pcscd is its only requirement; the
#               graphics libraries (OpenGL, X11/Wayland) are loaded at run time, so it can't be static
#
#   packaging/linux/build.sh
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$root/target/linux-dist"

# Each build has its own target dir: pcsc-sys doesn't rebuild when PCSC_LIB_* change, so a dir that
# once linked libpcsclite dynamically would keep doing so.
docker run --rm -v "$root":/src -w /src -e CARGO_TARGET_DIR=/src/target/linux-musl rust:1.99-alpine sh -euc '
    apk add -q build-base meson ninja flex curl xz > /dev/null
    lib=$(packaging/linux/pcsclite.sh /tmp/pcsclite)
    PCSC_LIB_DIR=$lib PCSC_LIB_NAME=static=pcsclite cargo build --release --locked -p eid-cli
    # A static binary has no NEEDED entries (ldd is no test: it runs static PIEs on x86_64).
    if readelf -d target/linux-musl/release/eid | grep -q NEEDED; then echo "eid is not static" >&2; exit 1; fi
    cp target/linux-musl/release/eid target/linux-dist/'

docker run --rm -v "$root":/src -w /src -e CARGO_TARGET_DIR=/src/target/linux-glibc rust:1.99-bookworm sh -euc '
    apt-get update -qq && apt-get install -y -qq --no-install-recommends meson ninja-build flex xz-utils > /dev/null
    lib=$(packaging/linux/pcsclite.sh /tmp/pcsclite)
    PCSC_LIB_DIR=$lib PCSC_LIB_NAME=static=pcsclite cargo build --release --locked -p eid-viewer
    if readelf -d target/linux-glibc/release/eid-viewer | grep NEEDED | grep -q pcsclite; then
        echo "libpcsclite is not linked in" >&2; exit 1
    fi
    cp target/linux-glibc/release/eid-viewer target/linux-dist/'

echo "$root/target/linux-dist"
