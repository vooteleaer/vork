#!/bin/sh
# vorkd, then the extractor for $EXTRACT_NS (its app token made once, kept
# in the store), then the web UI if VORK_WEB_LISTEN is set. The vØrk node is
# the compose file's other container, reached through $VORK_API.
# A stop (SIGTERM from docker) is passed on, so vorkd closes its store
# cleanly; sh as PID 1 wouldn't (vorkd would be killed after the grace).
set -e
mkdir -p "$VORK_STORE"
# A core, if the watchdog aborts a stalled daemon, lands in the store's volume.
cd "$VORK_STORE"
vorkd --store "$VORK_STORE" --no-mounts ${DEVICE:+--device "$DEVICE"} &
pid=$!
xpid=
wpid=
stop() {
    [ -n "$wpid" ] && kill -TERM "$wpid" 2>/dev/null
    [ -n "$xpid" ] && kill -TERM "$xpid" 2>/dev/null
    kill -TERM "$pid" 2>/dev/null
    wait "$pid"
    exit 0
}
trap stop TERM INT
i=0
until vork ping >/dev/null 2>&1; do
    i=$((i + 1)); [ $i -gt 600 ] && { echo "vorkd didn't come up"; exit 1; }
    sleep 1
done
echo "vorkd is up: $(vork ping 2>&1 | head -1)"
if [ -n "$EXTRACT_NS" ]; then
    T="$VORK_STORE/extract.token"
    if [ ! -s "$T" ]; then
        for ns in $(echo "$EXTRACT_NS" | tr ',' ' '); do vork ns info "$ns" >/dev/null 2>&1 || vork ns address "$ns" >/dev/null; done
        vork app add extract --ns "$EXTRACT_NS" --write \
            --keys "mime,width,height,taken,camera,geo,duration,exif.*,audio.*,doc.*,block:*,@content,type,extracted" > "$T"
    fi
    vork-extractd --token-file "$T" --every 30 &
    xpid=$!
fi
# The web UI (ADR-0037): VORK_WEB_LISTEN, VORK_WEB_ALLOW, VORK_WEB_PAIRING.
if [ -n "$VORK_WEB_LISTEN" ]; then
    vork-web &
    wpid=$!
fi
# Waits until vorkd ends, or a signal runs stop().
wait "$pid"
