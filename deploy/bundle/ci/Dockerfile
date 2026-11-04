# CI only (ci.yml, bundle): the node's image, from an odeiond binary built on the runner
# (Ubuntu 24.04, so the same glibc), with the release image's entrypoint and user.
FROM ubuntu:24.04
# ffmpeg for download transcodes, as in the release image (Ubuntu 24.04's 6.1 series here)
ARG FFMPEG_VERSION=7:6.1.*
RUN apt-get update && apt-get install -y --no-install-recommends libasound2t64 ca-certificates \
        "ffmpeg=${FFMPEG_VERSION}" \
    && rm -rf /var/lib/apt/lists/* \
    && (userdel -r ubuntu 2>/dev/null || true) && useradd -r -u 1000 -g audio -d /data pm
COPY odeiond /usr/local/bin/odeiond
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
# pm-node: the pre-Odeion name, a link as in the release image (backup.sh runs it)
RUN chmod +x /usr/local/bin/entrypoint.sh /usr/local/bin/odeiond && ln -s odeiond /usr/local/bin/pm-node && mkdir -p /data /music && chown pm:audio /data
USER pm
VOLUME ["/data", "/music"]
ENV PM_LISTEN=0.0.0.0:7788 PM_DATA_DIR=/data PM_FOLDERS=/music RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["serve"]
