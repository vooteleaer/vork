# vork: vorkd, vork (CLI), vork-extractd and vork-web, from binaries
# built beforehand (bin/amd64, bin/arm64; see ../README.md), for
# `docker buildx build --platform linux/amd64,linux/arm64`.
#
# bin/ also holds prnsd, the same as the Reticulum image's, only as a control
# command: the web UI asks it for Reticulum's state and has it apply
# configuration changes, through the files in Reticulum's volume. It's never
# started as a daemon here: Reticulum is the other container's (or the host's).
FROM debian:trixie-slim
ARG TARGETARCH
COPY bin/${TARGETARCH}/ /usr/local/bin/
COPY entry.sh /entry.sh
ENV VORK_STORE=/data/store \
    RETICULUM_CONFIG_DIR=/var/lib/prnsd \
    PRNSD_STATE_DIR=/var/lib/prnsd/.service
RUN mkdir -p /data/store && chown 65532:65532 /data/store
USER 65532:65532
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=10s --start-period=120s --retries=3 CMD ["vork", "ping"]
CMD ["sh", "/entry.sh"]
