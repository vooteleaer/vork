# Reticulum: prnsd (Prns) with the hardware interface families (RNode,
# serial, KISS, AutoInterface, USB, BLE; Prns's own image builds the cloud
# profile only, which has none of them), from binaries built beforehand
# (bin/amd64, bin/arm64; see ../README.md), for
# `docker buildx build --platform linux/amd64,linux/arm64`.
# Runs with `--service`, so other containers sharing its volume can ask it
# for its state and have it apply configuration changes (prnsd's control
# files in $PRNSD_STATE_DIR).
FROM debian:trixie-slim
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends libdbus-1-3 ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -m 0700 -o 65532 -g 65532 /var/lib/prnsd
COPY bin/${TARGETARCH}/prnsd /usr/local/bin/prnsd
USER 65532:65532
ENV PRNSD_STATE_DIR=/var/lib/prnsd/.service
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/usr/local/bin/prnsd", "status", "--json", "--config", "/var/lib/prnsd"]
ENTRYPOINT ["/usr/local/bin/prnsd"]
CMD ["run", "--service", "--config", "/var/lib/prnsd"]
