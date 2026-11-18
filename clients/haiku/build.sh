#!/bin/sh
# Builds vork for Haiku (the status and settings utility) on Haiku:
# the core as a library (haiku/capi), the BeAPI app over it, put together
# as an app folder (haiku/build/vork: the app, lib/libvork_desktop.so).
#
#     haiku/build.sh
#
# Needs: pkgman install rust_bin gcc haiku_devel. Run in the vork repository.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT=$HERE/build/vork
cd "$ROOT"
cargo build --release --ignore-rust-version -p vork-desktop-capi
TARGET=$ROOT/../target
mkdir -p "$OUT/lib"
# A new file, not the old one overwritten: a running app has it mapped.
rm -f "${OUT:?}/lib/libvork_desktop.so"
cp "$TARGET/release/libvork_desktop.so" "$OUT/lib/"
# $ORIGIN/lib: the core is found next to the app also when Deskbar, not the
# app, loads it (the replicant); -rdynamic: Deskbar finds the replicant's
# Instantiate() among the app's exported symbols.
g++ -O2 -std=c++17 -Wall -o "$OUT/vork" "$HERE/app/VorkApp.cpp" \
	-I "$HERE/capi" -I /boot/system/develop/headers/private/shared \
	-L "$OUT/lib" -lvork_desktop -lbe -lshared \
	-Wl,-rpath,'$ORIGIN/lib' -rdynamic
rc -o "$HERE/build/retifs.rsrc" "$HERE/app/retifs.rdef"
xres -o "$OUT/vork" "$HERE/build/retifs.rsrc"
mimeset -f "$OUT/vork"
# The programs it manages, in its bin/ folder as a package has them: the vØrk
# node (vork), vØrk storage (vorkd) and its commands (vork-storage), built
# in this repository (cargo build --release -p vork -p vork-storage-node).
rm -f "${OUT:?}/bin/prnsd"
for p in "$ROOT/../target/release/vorkd" "$ROOT/../target/release/vork" "$ROOT/../target/release/vork-storage"; do
	if [ -f "$p" ]; then
		mkdir -p "$OUT/bin"
		rm -f "${OUT:?}/bin/$(basename "$p")"	# a running one keeps its old file
		cp "$p" "$OUT/bin/"
	fi
done
# The libraries a new install joins (vorkd reads it next to itself).
mkdir -p "$OUT/bin"
cp "$ROOT/../packaging/default-libraries" "$OUT/bin/default-libraries"
# Libraries as volumes: the userlandfs module (vork-haikufs), when built.
M="$ROOT/../target/release/libvork_haikufs.so"
if [ -f "$M" ]; then
	mkdir -p "$OUT/add-ons"
	rm -f "${OUT:?}/add-ons/vork-haikufs"
	cp "$M" "$OUT/add-ons/vork-haikufs"
fi
echo "== $OUT"
ls -la "$OUT" "$OUT/lib" "$OUT/bin"
