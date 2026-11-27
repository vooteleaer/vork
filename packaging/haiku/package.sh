#!/bin/sh
# Makes vork-<version>-x86_64.hpkg: vØrk the network for Haiku (ADR-0029).
# On Haiku, after the node is built:
#
#     cargo build --release -p vork      # with Haiku's mio fix, see below
#     packaging/haiku/package.sh [target/release/vork]
#     pkgman install vork-<version>-x86_64.hpkg
#
# The node runs for each user from login (a launch_daemon job), its settings
# and identity in ~/config/settings/vork; its dashboard opens in WebPositive
# from the Deskbar's Applications menu. Its interface (tun/0, Haiku's
# tunnel driver) is IPv4 only, as Haiku's network: programs there reach your
# devices at their 198.18 addresses; .vork names don't resolve there yet.
#
# Four crates need a change for Haiku until their releases have it; build
# with a copy of each from ~/.cargo/registry, changed, given as
#   --config 'patch.crates-io.<crate>.path="<the copy>"':
# - mio (1.2.5): Haiku in the O_NONBLOCK-after-accept lists of
#   src/sys/unix/tcp.rs and uds/listener.rs (tokio-rs/mio#2036): without it
#   the dashboard's and the control socket's connections block the node;
# - if-addrs (0.15.0): Haiku left out of its netlink change notifier, as the
#   BSDs (every not(target_os = "illumos") / target_os = "illumos" list);
# - boringtun (0.7.1): CLOCK_MONOTONIC for CLOCK_BOOTTIME in
#   src/sleepyinstant/unix.rs (Haiku has no CLOCK_BOOTTIME);
# - ipstack (1.0.1): target_os = "haiku" in src/lib.rs's lists with linux
#   (its TUN_PROTO constants, for a packet header the node doesn't use).
# mDNS (local network discovery) is left out on Haiku: mdns-sd needs IP_PKTINFO.
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
BIN=${1:-$ROOT/target/release/vork}
[ -x "$BIN" ] || { echo "no node at $BIN: build it first" >&2; exit 1; }
OUT=${OUT:-$ROOT/target/haiku}
STAGE=$OUT/package
ARCH=x86_64

# The version from the binary (0.2.0, or 0.3.0-dev.140); Haiku's needs a
# revision after a dash and no other dashes: 0.2.0-1, 0.3.0~dev.140-1.
FULL=$("$BIN" version | awk '{print $2}')
VERSION=$(echo "$FULL" | sed 's/-/~/')-${REVISION:-1}

rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/apps/vork" "$STAGE/data/user_launch" "$STAGE/data/deskbar/menu/Applications"
cp "$BIN" "$STAGE/bin/vork"
strip "$STAGE/bin/vork" 2>/dev/null || true

# The node, for each user: its configuration made at the first start (a free
# UDP port, asked of the router; the dashboard on this machine only).
cat > "$STAGE/apps/vork/vork-node" <<'EOF'
#!/bin/sh
C=$HOME/config/settings/vork/vork.toml
if [ ! -e "$C" ]; then
	/bin/vork init --config "$C" --port auto --tun tun/0
	sed -i 's|^# web = .*|web = "127.0.0.1:8742"|' "$C"
fi
exec /bin/vork run --config "$C"
EOF
# The dashboard, in the browser, with the key the node keeps.
cat > "$STAGE/apps/vork/vØrk" <<'EOF'
#!/bin/sh
L=$(/bin/vork web-link --config "$HOME/config/settings/vork/vork.toml" 2>/dev/null | head -1)
if [ -n "$L" ]; then
	open "$L"
else
	alert --stop "vØrk's node isn't running yet: it starts at login (or launch_roster start x-vnd.vork-node)."
fi
EOF
chmod 755 "$STAGE/apps/vork/vork-node" "$STAGE/apps/vork/vØrk"
ln -s ../../../../apps/vork/vØrk "$STAGE/data/deskbar/menu/Applications/vØrk"

cat > "$STAGE/data/user_launch/vork" <<'EOF'
# vØrk: the node, for every user from login.
job x-vnd.vork-node {
	launch /boot/system/apps/vork/vork-node
}
EOF

cat > "$STAGE/.PackageInfo" <<EOF
name			vork
version			$VERSION
architecture	$ARCH
summary			"vØrk: your own private network between your devices"
description		"vØrk connects your own devices into one private network, with no account \
and no company in the middle: they reach each other from anywhere, through a \
network of equal nodes where any node can relay and none can read what it carries.

The node starts at login; its dashboard is in the Deskbar's Applications menu. \
Programs here reach your devices at their vØrk IPv4 addresses (198.18.x.x) \
through its interface; .vork names don't resolve here yet. An early prototype."
packager		"vØrk <https://github.com/vooteleaer/vork>"
vendor			"vØrk"
licenses {
	"MIT"
}
copyrights {
	"2026 vØrk contributors"
}
provides {
	vork = $VERSION
	cmd:vork
}
requires {
	haiku
	lib:libgcc_s
}
urls {
	"https://github.com/vooteleaer/vork"
}
EOF

PKG=$OUT/vork-$VERSION-$ARCH.hpkg
rm -f "$PKG"
package create -C "$STAGE" "$PKG"
echo "== $PKG"
package list "$PKG"
