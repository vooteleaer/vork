#!/bin/sh
# Builds the image for linux/amd64 and linux/arm64 from binaries built
# beforehand. Usage: build.sh BIN_DIR [VERSION] [--push]
#   BIN_DIR/{amd64,arm64}/{vork,vorkd,vork-storage,vork-extractd,vork-web}
# Without --push the image is built (to check it) and not kept: buildx
# can't load a multi-platform image; with it, it's published as
# vooteleaer/vork:VERSION (+latest).
set -e
BIN=${1:?BIN_DIR}
VERSION=${2:-dev}
PUSH=${3:-}
HERE=$(cd "$(dirname "$0")" && pwd)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
OUT="--output=type=cacheonly"
[ "$PUSH" = "--push" ] && OUT="--push"

mkdir -p "$STAGE/vork/bin"
for a in amd64 arm64; do
    mkdir -p "$STAGE/vork/bin/$a"
    for f in vork vorkd vork-storage vork-extractd vork-web; do cp "$BIN/$a/$f" "$STAGE/vork/bin/$a/"; done
done
cp "$HERE/vork/Dockerfile" "$HERE/vork/entry.sh" "$HERE/vork/node.sh" "$STAGE/vork/"

TAGS="-t vooteleaer/vork:$VERSION"
[ "$VERSION" != dev ] && TAGS="$TAGS -t vooteleaer/vork:latest"
docker buildx build --platform linux/amd64,linux/arm64 $OUT $TAGS "$STAGE/vork"
