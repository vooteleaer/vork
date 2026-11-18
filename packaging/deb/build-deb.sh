#!/bin/bash
# Builds vork_<version>_<arch>.deb for amd64 and arm64 (Debian, Ubuntu,
# Raspberry Pi OS): the vØrk node (a system service) and vØrk storage (vorkd
# and its helpers, user services).
#
#     packaging/deb/build-deb.sh [OUT_DIR] [ARCH...]
#
# On a Debian or Ubuntu x86_64 machine (WSL works) with Rust, dpkg-deb and
# `rustup target add aarch64-unknown-linux-gnu`. With cargo-zigbuild (and
# zig) the binaries are linked against glibc $GLIBC (default 2.31: Debian 11,
# Ubuntu 20.04 and 22.04, elementary OS 6 and 7, Raspberry Pi OS bullseye and
# later), whatever the build machine has; without it,
# against the build machine's glibc, and arm64 needs gcc-aarch64-linux-gnu.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
OUT=${1:-$ROOT/target/deb}
shift || true
ARCHS=${*:-amd64 arm64}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/crates/vork-storage-node/Cargo.toml" | head -1 | tr -d '\r')
COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)
# A release when this commit is tagged with the version, else a build towards
# it; the commit count keeps those in order (vork docs/storage/versioning.md).
COUNT=$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null || echo 0)
if [ "$(git -C "$ROOT" describe --exact-match --tags HEAD 2>/dev/null)" = "v$VERSION" ]; then
    DEBVER=$VERSION
else
    DEBVER="$VERSION~dev.$COUNT.$COMMIT"
fi
BINS="vorkd vork vork-storage vork-mount vork-gateway vork-extractd"
GLIBC=${GLIBC:-2.31}
if command -v cargo-zigbuild >/dev/null; then
    BUILD="cargo zigbuild"; SUFFIX=".$GLIBC"
else
    BUILD="cargo build"; SUFFIX=""; GLIBC=$(ldd --version | sed -n '1s/.* //p')
    # zig links for any target itself; without it, arm64 needs the cross gcc.
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:-aarch64-linux-gnu-gcc}
fi
mkdir -p "$OUT"

for ARCH in $ARCHS; do
    case $ARCH in
        amd64) TARGET=x86_64-unknown-linux-gnu ;;
        arm64) TARGET=aarch64-unknown-linux-gnu ;;
        *) echo "unknown arch $ARCH" >&2; exit 2 ;;
    esac
    echo "== $ARCH: building vork"
    $BUILD --release --locked --target $TARGET$SUFFIX --manifest-path "$ROOT/Cargo.toml" \
        -p vork -p vork-storage-node -p vork-fuse -p vork-gateway -p vork-extract
    BIN=$CARGO_TARGET_DIR/$TARGET/release

    STAGE=$(mktemp -d)
    chmod 0755 "$STAGE"
    trap 'rm -rf "$STAGE"' EXIT
    install -d "$STAGE/DEBIAN" "$STAGE/usr/bin" "$STAGE/usr/lib/systemd/system" "$STAGE/usr/lib/systemd/user" "$STAGE/usr/share/doc/vork"
    for b in $BINS; do
        install -m 0755 "$BIN/$b" "$STAGE/usr/bin/$b"
    done
    # The libraries a new install joins (vorkd reads ../share/vork/ from /usr/bin).
    install -D -m 0644 "$ROOT/packaging/default-libraries" "$STAGE/usr/share/vork/default-libraries"
    # The user units, pointed at the installed binaries.
    # Not the user's own node (systemd/vork.service): the package's is a system service.
    for u in "$ROOT"/packaging/systemd/*.service; do
        [ "$(basename "$u")" = vork.service ] && continue
        sed -e 's|%h/.cargo/bin/|/usr/bin/|g' -e '/^#   cp packaging/d' "$u" > "$STAGE/usr/lib/systemd/user/$(basename "$u")"
    done
    # The node, a system service (its sockets in /var/lib/vork are every user's).
    sed -e 's|/usr/local/bin/vork|/usr/bin/vork|g' -e '/^#   /d' -e '/^#$/d' "$ROOT/packaging/vork.service" \
        > "$STAGE/usr/lib/systemd/system/vork.service"
    DOC=$STAGE/usr/share/doc/vork
    install -m 0644 "$ROOT/LICENSE" "$DOC/copyright"
    install -m 0644 "$HERE/README.Debian" "$DOC/"

    SIZE=$(du -sk "$STAGE/usr" | cut -f1)
    cat > "$STAGE/DEBIAN/control" <<EOF
Package: vork
Version: $DEBVER
Architecture: $ARCH
Maintainer: vork <https://github.com/vooteleaer/vork>
Installed-Size: $SIZE
Depends: libc6 (>= $GLIBC), fuse3
Suggests: ffmpeg
Section: net
Priority: optional
Homepage: https://github.com/vooteleaer/vork
Description: serverless encrypted network and distributed data (prototype)
 vØrk: a network of equal nodes over WireGuard-style UDP, addressed by
 identity, where any node can relay. On it, records and libraries instead
 of files, synced between your devices and others', encrypted end to end.
 A working prototype: the formats still change, keep no data in it you
 can't lose.
 .
 Includes the vØrk node (vork, a system service), vorkd, the vork-storage
 commands, the FUSE mount, the local web gateway and the attribute
 extractor.
EOF
    install -m 0755 "$HERE/postinst" "$HERE/prerm" "$STAGE/DEBIAN/"
    DEB="$OUT/vork_${DEBVER}_${ARCH}.deb"
    dpkg-deb --root-owner-group -Zxz --build "$STAGE" "$DEB" >/dev/null
    rm -rf "$STAGE"
    trap - EXIT
    echo "== $DEB ($(du -h "$DEB" | cut -f1))"
done
