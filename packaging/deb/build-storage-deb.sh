#!/bin/sh
# Builds vork-storage_<version>_<arch>.deb: vØrk storage for Debian and
# Ubuntu, for this machine's architecture. It needs vØrk the network (the
# vork package) and runs for each user from login (a systemd user service).
#
#     packaging/deb/build-storage-deb.sh [OUT]          # on Linux, in the repository
#     packaging/deb/build-in-docker.sh --storage        # both architectures, in Ubuntu 22.04
#
# Holds vorkd, vork-web (its pages in the vØrk dashboard), vork-mount (the
# libraries as folders in ~/vork, FUSE), the vork-storage command and
# vork-storage-run, which keeps them running. Needs Rust and dpkg-dev.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT=${1:-$ROOT/target/deb}
ARCH=$(dpkg --print-architecture)
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target/deb-build-$ARCH}

# The version as vØrk's (ADR-0029): storage is released with it.
VERSION=$(awk '/^\[package\]/{p=1} p&&/^version = /{gsub(/"|\r/,"",$3); print $3; exit}' "$ROOT/Cargo.toml")
COUNT=$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || echo 0)
COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)
if [ "$(git -C "$ROOT" tag --points-at HEAD 2>/dev/null | grep -x "v$VERSION")" = "v$VERSION" ]; then
    DEBVER=$VERSION
else
    DEBVER="$VERSION~dev.$COUNT.$COMMIT"   # ~ sorts below the release
fi

echo "== vork-storage $DEBVER ($ARCH)"
cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" -p vork-storage-node -p vork-web -p vork-fuse
BIN=$CARGO_TARGET_DIR/release

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
chmod 0755 "$STAGE"
install -d "$STAGE/DEBIAN" "$STAGE/usr/bin" "$STAGE/usr/lib/systemd/user" "$STAGE/usr/share/doc/vork-storage"
for b in vorkd vork-storage vork-storage-run vork-web vork-mount; do
    install -m 0755 -s "$BIN/$b" "$STAGE/usr/bin/$b"
done
install -m 0644 "$HERE/storage/vork-storage.service" "$STAGE/usr/lib/systemd/user/vork-storage.service"
install -m 0644 "$ROOT/LICENSE" "$STAGE/usr/share/doc/vork-storage/copyright"

NEEDED=$(for b in "$STAGE"/usr/bin/*; do readelf -d "$b" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'; done | sort -u)
DEPS=$(for n in $NEEDED; do
        p=$(ldconfig -p | awk -v n="$n" '$1 == n && /x86-64|AArch64|aarch64/ {print $NF; exit}')
        [ -n "$p" ] && { dpkg -S "$(readlink -f "$p")" 2>/dev/null || dpkg -S "$p" 2>/dev/null; }
    done | cut -d: -f1 | sort -u | grep -v '^libc6$' | paste -sd, - | sed 's/,/, /g')
GLIBC=$(ldd --version | sed -n '1s/.* //p')
SIZE=$(du -sk "$STAGE/usr" | cut -f1)
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: vork-storage
Version: $DEBVER
Architecture: $ARCH
Maintainer: vØrk <https://github.com/vooteleaer/vork>
Installed-Size: $SIZE
Depends: libc6 (>= $GLIBC)${DEPS:+, $DEPS}, vork, fuse3
Section: utils
Priority: optional
Homepage: https://github.com/vooteleaer/vork
Description: your files on your own devices, over vØrk (prototype)
 vØrk storage keeps libraries of files (photos, music, documents) on your
 own devices, encrypted, copied between them over vØrk: no cloud and no
 account. Each library is a folder in ~/vork; its pages are in the vØrk
 dashboard.
 .
 Runs for each user from login (a systemd user service). Needs vØrk the
 network (the vork package). A prototype: the storage format still changes
 between versions.
CONTROL
install -m 0755 "$HERE/storage/postinst" "$HERE/storage/prerm" "$STAGE/DEBIAN/"
mkdir -p "$OUT"
DEB="$OUT/vork-storage_${DEBVER}_${ARCH}.deb"
dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB" >/dev/null
echo "== $DEB ($(du -h "$DEB" | cut -f1))"
echo "   Depends: libc6 (>= $GLIBC)${DEPS:+, $DEPS}, vork, fuse3"
