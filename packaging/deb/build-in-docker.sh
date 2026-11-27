#!/bin/sh
# Builds the .deb for amd64 and arm64 in Ubuntu 22.04 containers (arm64
# through Docker's emulation: slow, but no cross WebKitGTK is needed).
#
#     packaging/deb/build-in-docker.sh [amd64] [arm64]
#
# The packages land in target/deb (vØrk storage's are built in its own
# repository). Each architecture keeps its own Cargo cache and build folder
# in target/, so a second run is quicker.
set -e
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SCRIPT=build-deb.sh
PACKAGES="libwebkit2gtk-4.1-dev libayatana-appindicator3-dev libxdo-dev"
for ARCH in ${*:-amd64 arm64}; do
    docker run --rm --platform "linux/$ARCH" \
        -v "$ROOT:/src" -w /src \
        -v "$ROOT/target/deb-cargo-$ARCH:/root/.cargo/registry" \
        -e CARGO_TARGET_DIR="/src/target/deb-build-$ARCH" \
        -e SCRIPT="$SCRIPT" -e PACKAGES="$PACKAGES" \
        ubuntu:22.04 sh -c '
            set -e
            export DEBIAN_FRONTEND=noninteractive
            apt-get update -qq
            apt-get install -y -qq --no-install-recommends curl ca-certificates build-essential pkg-config git dpkg-dev $PACKAGES >/dev/null
            curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal >/dev/null
            . /root/.cargo/env
            git config --global --add safe.directory /src
            packaging/deb/$SCRIPT /src/target/deb'
done
ls -l "$ROOT/target/deb"
