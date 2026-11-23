#!/bin/sh
# Makes the vork package for Haiku (an hpkg) from the app folder
# build.sh made: the app with its library, vorkd and the vork command
# (on the PATH), vorkd as a launch_daemon job at login, the userlandfs
# module that makes each library a volume (vorkd mounts every library in
# ~/vork once the desktop is up), and an entry in the Deskbar's
# Applications menu. Not mounted by a login job: a userlandfs mount from one
# blocked the user's launch_daemon, and the desktop never came up. The vØrk
# node (vork) runs as a launch_daemon job of its own, before vorkd.
#
#     haiku/build.sh && haiku/package.sh
#     pkgman install haiku/build/vork-<version>-x86_64.hpkg
#
# A system package: the launch jobs name the programs at their place under
# /boot/system. On Haiku, with vorkd built in vork.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
APP=$HERE/build/vork
STAGE=$HERE/build/package
# The revision: the commit count (vork docs/storage/versioning.md), given as REVISION
# where the sources are copied without .git.
REVISION=${REVISION:-$(git -C "$ROOT" rev-list --count HEAD 2>/dev/null)}
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1 | tr -d '\r')-${REVISION:?no REVISION (the commit count)}
ARCH=x86_64

for f in vork lib/libvork_desktop.so bin/vorkd bin/vork bin/vork-storage add-ons/vork-haikufs; do
	[ -f "$APP/$f" ] || { echo "missing $APP/$f: build it first" >&2; exit 1; }
done

rm -rf "${STAGE:?}"
mkdir -p "$STAGE/apps" "$STAGE/bin" "$STAGE/data/user_launch" \
	"$STAGE/data/deskbar/menu/Applications" "$STAGE/add-ons/userlandfs"
cp -a "$APP" "$STAGE/apps/vork"
# The module where userlandfs looks for it, by the name mount -t userlandfs takes.
mv "$STAGE/apps/vork/add-ons/vork-haikufs" "$STAGE/add-ons/userlandfs/vork"
rmdir "$STAGE/apps/vork/add-ons"
# A prnsd from an older build folder: Prns's package has it now.
rm -f "$STAGE/apps/vork/bin/prnsd"
ln -s ../apps/vork/bin/vork "$STAGE/bin/vork"
ln -s ../apps/vork/bin/vork-storage "$STAGE/bin/vork-storage"
ln -s ../../../../apps/vork/vork "$STAGE/data/deskbar/menu/Applications/vork"

# The jobs, with the package's paths (launch/vork is the template).
{
	echo "# vork (package): the vØrk node and the vork daemon, started for"
	echo "# every user at login."
	sed 's|APP|/boot/system/apps/vork|g' "$HERE/launch/vork" | grep -v '^#'
} > "$STAGE/data/user_launch/vork"
cat >> "$STAGE/data/user_launch/vork" <<'EOF'

# The status icon in the Deskbar's tray (added once; the Deskbar keeps it):
# with the desktop, once the Deskbar runs. At top level it started before
# app_server, and a login sometimes never got its desktop (2026-11-09).
target desktop {
	job x-vnd.vork-storage-deskbar {
		launch /boot/system/apps/vork/vork --deskbar
		requires x-vnd.Be-TSKB
	}
}
EOF

cat > "$STAGE/.PackageInfo" <<EOF
name			vork
version			$VERSION
architecture	$ARCH
summary			"Distributed, always-encrypted data over vØrk"
description		"vork keeps records and libraries instead of files, synced between your \
devices and others' over the vØrk network, encrypted end to end. A working prototype: \
the formats still change, keep no data in it you can't lose.

Includes the vØrk node, the vork daemon and commands, and the status and \
settings utility with its Deskbar icon. Both start at login as launch_daemon \
jobs; the store is ~/config/var/vork."
packager		"vork <https://github.com/vooteleaer/vork>"
vendor			"vork"
licenses {
	"MIT"
}
copyrights {
	"2026 vork contributors"
}
provides {
	vork = $VERSION
	app:vork = $VERSION
	cmd:vork
	cmd:vork_storage
}
requires {
	haiku
	userland_fs
	lib:libgcc_s
	lib:libstdc++
}
urls {
	"https://github.com/vooteleaer/vork"
}
EOF

PKG=$HERE/build/vork-$VERSION-$ARCH.hpkg
rm -f "$PKG"
cd "$STAGE"
package create -C "$STAGE" "$PKG"
echo "== $PKG"
package list "$PKG" | head -30
