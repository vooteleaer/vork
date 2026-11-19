#!/bin/sh
# Builds vork_<version>_<arch>.deb: vØrk the network for Debian and Ubuntu
# (ADR-0029), for this machine's architecture. Storage isn't part of it.
#
#     packaging/deb/build-deb.sh [OUT]       # on Linux, in the repository
#     packaging/deb/build-in-docker.sh       # both architectures, in Ubuntu 22.04
#
# Holds the node (a system service), vork-net (the window and tray icon,
# clients/net-desktop) and its desktop entries. Needs Rust, dpkg-dev and
# vork-net's build dependencies (libwebkit2gtk-4.1-dev
# libayatana-appindicator3-dev libxdo-dev). Built on Ubuntu 22.04, it runs
# on Debian 12 and Ubuntu 22.04 or newer (glibc and WebKitGTK 4.1).
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT=${1:-$ROOT/target/deb}
ARCH=$(dpkg --print-architecture)
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target/deb-build-$ARCH}

# The version (ADR-0029): the node crate's; a release's when this commit is
# tagged with it, else a build towards it, counted in commits.
VERSION=$(awk '/^\[package\]/{p=1} p&&/^version = /{gsub(/"|\r/,"",$3); print $3; exit}' "$ROOT/Cargo.toml")
COUNT=$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || echo 0)
COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)
if [ "$(git -C "$ROOT" tag --points-at HEAD 2>/dev/null | grep -x "v$VERSION")" = "v$VERSION" ]; then
    DEBVER=$VERSION
else
    DEBVER="$VERSION~dev.$COUNT.$COMMIT"   # ~ sorts below the release
fi

echo "== vork $DEBVER ($ARCH)"
cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" -p vork
cargo build --release --locked --manifest-path "$ROOT/clients/net-desktop/Cargo.toml" --target-dir "$CARGO_TARGET_DIR/net-desktop"
NODE=$CARGO_TARGET_DIR/release/vork
NET=$CARGO_TARGET_DIR/net-desktop/release/vork-net

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
chmod 0755 "$STAGE"
install -d "$STAGE/DEBIAN" "$STAGE/usr/bin" "$STAGE/usr/lib/systemd/system" "$STAGE/usr/share/doc/vork" \
    "$STAGE/usr/share/applications" "$STAGE/usr/share/icons/hicolor/scalable/apps" "$STAGE/etc/xdg/autostart"
install -m 0755 -s "$NODE" "$STAGE/usr/bin/vork"
install -m 0755 -s "$NET" "$STAGE/usr/bin/vork-net"
# The node, a system service; its control socket in /var/lib/vork.
sed -e 's|/usr/local/bin/vork|/usr/bin/vork|g' -e '/^#   /d' -e '/^#$/d' "$ROOT/packaging/vork.service" \
    > "$STAGE/usr/lib/systemd/system/vork.service"
install -m 0644 "$ROOT/packaging/linux/vork-net.desktop" "$STAGE/usr/share/applications/vork-net.desktop"
install -m 0644 "$ROOT/packaging/linux/vork-net-autostart.desktop" "$STAGE/etc/xdg/autostart/vork-net.desktop"
install -m 0644 "$ROOT/packaging/linux/vork.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/vork.svg"
install -m 0644 "$ROOT/LICENSE" "$STAGE/usr/share/doc/vork/copyright"
install -m 0644 "$HERE/README.Debian" "$STAGE/usr/share/doc/vork/"
echo /etc/xdg/autostart/vork-net.desktop > "$STAGE/DEBIAN/conffiles"

# Depends: the packages holding the libraries the programs link to directly
# (NEEDED, not ldd's whole tree: those names differ between distributions).
# The tray's app indicator library is loaded at run time: Recommends.
NEEDED=$(for b in "$NODE" "$NET"; do readelf -d "$b" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'; done | sort -u)
DEPS=$(for n in $NEEDED; do
        p=$(ldconfig -p | awk -v n="$n" '$1 == n && /x86-64|AArch64|aarch64/ {print $NF; exit}')
        [ -n "$p" ] && { dpkg -S "$(readlink -f "$p")" 2>/dev/null || dpkg -S "$p" 2>/dev/null; }
    done | cut -d: -f1 | sort -u | grep -v '^libc6$' | paste -sd, - | sed 's/,/, /g')
GLIBC=$(ldd --version | sed -n '1s/.* //p')
SIZE=$(du -sk "$STAGE/usr" | cut -f1)
cat > "$STAGE/DEBIAN/control" <<EOF
Package: vork
Version: $DEBVER
Architecture: $ARCH
Maintainer: vØrk <https://github.com/vooteleaer/vork>
Installed-Size: $SIZE
Depends: libc6 (>= $GLIBC), $DEPS
Recommends: libayatana-appindicator3-1
Section: net
Priority: optional
Homepage: https://github.com/vooteleaer/vork
Description: your own private network between your devices (prototype)
 vØrk connects your devices so they reach each other from anywhere, by
 name (laptop.vork), through a network of equal nodes over WireGuard where
 any node can relay and nobody in the middle can read the traffic. No
 account and no central server. Only your linked devices reach yours.
 .
 Includes the vØrk node (vork, a system service) and vork-net, its window
 and tray icon. A prototype: the network format still changes between
 versions.
EOF
install -m 0755 "$HERE/postinst" "$HERE/prerm" "$STAGE/DEBIAN/"
mkdir -p "$OUT"
DEB="$OUT/vork_${DEBVER}_${ARCH}.deb"
dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB" >/dev/null
echo "== $DEB ($(du -h "$DEB" | cut -f1))"
echo "   Depends: libc6 (>= $GLIBC), $DEPS"
