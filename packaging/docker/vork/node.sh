#!/bin/sh
# The vØrk node: its configuration and identity made at the first start (in
# the volume), then run. SIGTERM from docker reaches it directly (exec).
set -e
C=${VORK_CONFIG:-/var/lib/vork/vork.toml}
[ -e "$C" ] || vork init --config "$C"
exec vork run --config "$C"
