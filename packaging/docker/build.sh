#!/bin/sh
# Builds the two images for linux/amd64 and linux/arm64 from binaries built
# beforehand. Usage: build.sh BIN_DIR [VERSION] [--push]
#   BIN_DIR/{amd64,arm64}/{vorkd,vork,vork-storage,vork-extractd,vork-web,prnsd}
# Without --push the images are built (to check them) and not kept: buildx
# can't load a multi-platform image; with it, they're published as
# vooteleaer/vork:VERSION (+latest) and vooteleaer/prnsd:0.3.8.
set -e
BIN=${1:?BIN_DIR}
VERSION=${2:-dev}
PUSH=${3:-}
HERE=$(cd "$(dirname "$0")" && pwd)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
OUT="--output=type=cacheonly"
[ "$PUSH" = "--push" ] && OUT="--push"

mkdir -p "$STAGE/reticulum/bin" "$STAGE/vork/bin"
for a in amd64 arm64; do
    mkdir -p "$STAGE/reticulum/bin/$a" "$STAGE/vork/bin/$a"
    cp "$BIN/$a/prnsd" "$STAGE/reticulum/bin/$a/"
    for f in vorkd vork vork-storage vork-extractd vork-web prnsd; do cp "$BIN/$a/$f" "$STAGE/vork/bin/$a/"; done
done
cp "$HERE/reticulum/Dockerfile" "$STAGE/reticulum/"
cp "$HERE/vork/Dockerfile" "$HERE/vork/entry.sh" "$STAGE/vork/"

docker buildx build --platform linux/amd64,linux/arm64 $OUT \
    -t vooteleaer/prnsd:0.3.8 "$STAGE/reticulum"
TAGS="-t vooteleaer/vork:$VERSION"
[ "$VERSION" != dev ] && TAGS="$TAGS -t vooteleaer/vork:latest"
docker buildx build --platform linux/amd64,linux/arm64 $OUT $TAGS "$STAGE/vork"
