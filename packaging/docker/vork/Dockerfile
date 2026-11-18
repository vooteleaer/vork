# vork: the vØrk node (vork), vorkd, vork-storage, vork-extractd and
# vork-web, from binaries built beforehand (bin/amd64, bin/arm64; see
# ../README.md), for `docker buildx build --platform linux/amd64,linux/arm64`.
# One image, two containers (../compose.yaml): the node (node.sh) and
# storage (entry.sh, the default).
FROM debian:trixie-slim
ARG TARGETARCH
COPY bin/${TARGETARCH}/ /usr/local/bin/
COPY entry.sh /entry.sh
COPY node.sh /node.sh
# The node's paths in its volume (vork init --config writes state/ beside
# the file), for the node and for storage's container.
ENV VORK_STORE=/data/store \
    VORK_CONFIG=/var/lib/vork/vork.toml \
    VORK_API=/var/lib/vork/state/api.sock \
    VORK_CONTROL=/var/lib/vork/state/control.sock
RUN mkdir -p /data/store /var/lib/vork && chown 65532:65532 /data/store /var/lib/vork
USER 65532:65532
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=10s --start-period=120s --retries=3 CMD ["vork", "ping"]
CMD ["sh", "/entry.sh"]
