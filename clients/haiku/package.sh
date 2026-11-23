#!/bin/sh
# Makes vork_storage-<version>-x86_64.hpkg: vØrk storage for Haiku, beside
# the vork package (vØrk the network, packaging/haiku), which it requires.
# From the app folder build.sh made: the status utility with its library,
# vorkd, vork-web (its pages in the vØrk dashboard), the vork-storage
# command (on the PATH), vork-storage-run as a launch_daemon job at login
# (after the node's), the userlandfs module that makes each library a volume
# (vorkd mounts every library in ~/vork once the desktop is up), and an
# entry in the Deskbar's Applications menu. Not mounted by a login job: a
# userlandfs mount from one blocked the user's launch_daemon, and the
# desktop never came up.
#
#     clients/haiku/build.sh && clients/haiku/package.sh
#     pkgman install clients/haiku/build/vork_storage-<version>-x86_64.hpkg
#
# A system package: the launch jobs name the programs at their place under
# /boot/system. On Haiku, in the vork repository.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
APP=$HERE/build/vork
STAGE=$HERE/build/package
ARCH=x86_64

# The version as vØrk's (ADR-0029): a release's when this commit is tagged
# with it, else a build towards it, counted in commits (given as REVISION
# where the sources are copied without .git). Haiku's has a revision after
# a dash and no other dashes: 0.4.0-1, 0.4.0~dev.180-1.
V=$(awk '/^\[package\]/{p=1} p&&/^version = /{gsub(/"|\r/,"",$3); print $3; exit}' "$REPO/Cargo.toml")
COUNT=${REVISION:-$(git -C "$REPO" rev-list --count HEAD 2>/dev/null)}
if [ "$(git -C "$REPO" tag --points-at HEAD 2>/dev/null | grep -x "v$V")" = "v$V" ]; then
	VERSION=$V-1
else
	VERSION=$V~dev.${COUNT:?no REVISION (the commit count)}-1
fi

for f in vork lib/libvork_desktop.so bin/vorkd bin/vork-storage bin/vork-storage-run bin/vork-web add-ons/vork-haikufs; do
	[ -f "$APP/$f" ] || { echo "missing $APP/$f: build it first" >&2; exit 1; }
done

rm -rf "${STAGE:?}"
mkdir -p "$STAGE/apps" "$STAGE/bin" "$STAGE/data/user_launch" \
	"$STAGE/data/deskbar/menu/Applications" "$STAGE/add-ons/userlandfs"
cp -a "$APP" "$STAGE/apps/vork-storage"
# The module where userlandfs looks for it, by the name mount -t userlandfs takes.
mv "$STAGE/apps/vork-storage/add-ons/vork-haikufs" "$STAGE/add-ons/userlandfs/vork"
rmdir "$STAGE/apps/vork-storage/add-ons"
# Left over in an older build folder: the node is the vork package's.
rm -f "$STAGE/apps/vork-storage/bin/prnsd" "$STAGE/apps/vork-storage/bin/vork" "$STAGE/apps/vork-storage/bin/default-libraries"
mv "$STAGE/apps/vork-storage/vork" "$STAGE/apps/vork-storage/vØrk storage"
ln -s ../apps/vork-storage/bin/vork-storage "$STAGE/bin/vork-storage"
ln -s "../../../../apps/vork-storage/vØrk storage" "$STAGE/data/deskbar/menu/Applications/vØrk storage"

# The jobs, with the package's paths (launch/vork-storage is the template).
{
	echo "# vØrk storage (package): started for every user at login, after the"
	echo "# vØrk node (the vork package's job)."
	sed 's|APP|/boot/system/apps/vork-storage|g' "$HERE/launch/vork-storage" | grep -v '^#'
} > "$STAGE/data/user_launch/vork-storage"
cat >> "$STAGE/data/user_launch/vork-storage" <<'EOF'

# The status icon in the Deskbar's tray (added once; the Deskbar keeps it):
# with the desktop, once the Deskbar runs. At top level it started before
# app_server, and a login sometimes never got its desktop (2026-11-09).
target desktop {
	job x-vnd.vork-storage-deskbar {
		launch "/boot/system/apps/vork-storage/vØrk storage" --deskbar
		requires x-vnd.Be-TSKB
	}
}
EOF

cat > "$STAGE/.PackageInfo" <<EOF
name			vork_storage
version			$VERSION
architecture	$ARCH
summary			"vØrk storage: your files on your own devices, over vØrk"
description		"vØrk storage keeps libraries of files (photos, music, documents) on your \
own devices, encrypted, copied between them over vØrk: no cloud and no account.

Each library is a volume in ~/vork; its pages are in the vØrk dashboard, and \
its status utility in the Deskbar's Applications menu. It starts at login, \
after the vØrk node; the store is ~/config/var/vork-storage. A prototype: the \
storage format still changes between versions."
packager		"vØrk <https://github.com/vooteleaer/vork>"
vendor			"vØrk"
licenses {
	"MIT"
}
copyrights {
	"2026 vØrk contributors"
}
provides {
	vork_storage = $VERSION
	app:vork_storage = $VERSION
	cmd:vork_storage
}
requires {
	haiku
	vork
	userland_fs
	lib:libgcc_s
	lib:libstdc++
}
urls {
	"https://github.com/vooteleaer/vork"
}
EOF

PKG=$HERE/build/vork_storage-$VERSION-$ARCH.hpkg
rm -f "$PKG"
package create -C "$STAGE" "$PKG"
echo "== $PKG"
package list "$PKG" | head -30
