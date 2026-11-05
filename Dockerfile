# The Odeion node: one static-ish binary (odeiond) with SQLite, ALSA output enabled. `pm-node`, its
# pre-Odeion name, stays as a link to it, so `docker exec <name> pm-node …` keeps working.
# Build:  docker build -t pubky-music-node .
# Multi-arch (buildx, cross-compiled on the build host, no QEMU for cargo):
#         docker buildx build --platform linux/amd64,linux/arm64 -t pubky-music-node .
# Run:    docker run --rm -p 7788:7788 --device /dev/snd -v music:/music:ro -v data:/data \
#           -e ODEION_CONTACT=you@example.org pubky-music-node   (token: docker exec <name> cat /data/api-token)
# The image keeps its pre-Odeion name (ghcr.io/gillohner/pubky-music-node): installs pull it by that name.

FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build
ARG TARGETARCH
# Native build for the host arch; for arm64 on an amd64 host cross-compile
# with the Debian multiarch ALSA headers and the GNU cross toolchain.
RUN set -eu; \
    apt-get update; \
    apt-get install -y --no-install-recommends pkg-config libasound2-dev; \
    if [ "$TARGETARCH" = "arm64" ] && [ "$(dpkg --print-architecture)" != "arm64" ]; then \
        dpkg --add-architecture arm64; \
        apt-get update; \
        apt-get install -y --no-install-recommends gcc-aarch64-linux-gnu libc6-dev-arm64-cross libasound2-dev:arm64 cmake; \
        rustup target add aarch64-unknown-linux-gnu; \
    fi; \
    rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
ENV CARGO_INCREMENTAL=0
# The version the node reports (release.yml: 0.2.1, or 0.2.1-edge+<sha>); empty = Cargo.toml's
ARG PM_NODE_VERSION
# Built-in apps (public ids), compiled in by crates/pm-api/build.rs (build stage only)
ARG PM_PROJECT_TIDAL_CLIENT_ID
ARG PM_PROJECT_JAMENDO_CLIENT_ID
ARG PM_PROJECT_ACOUSTID_KEY
ARG PM_PROJECT_FANART_KEY
ARG PM_PROJECT_AUDIODB_KEY
ARG PM_PROJECT_ONEDRIVE_CLIENT_ID
ARG PM_PROJECT_GOOGLE_CLIENT_ID
# The same values under their ODEION_ names, which win (pm_model::brand::build_env), also when passed
# empty. Declared without a default: one that is not passed stays unset, so the PM_ one still counts.
ARG ODEION_NODE_VERSION
ARG ODEION_PROJECT_TIDAL_CLIENT_ID
ARG ODEION_PROJECT_JAMENDO_CLIENT_ID
ARG ODEION_PROJECT_ACOUSTID_KEY
ARG ODEION_PROJECT_FANART_KEY
ARG ODEION_PROJECT_AUDIODB_KEY
ARG ODEION_PROJECT_ONEDRIVE_CLIENT_ID
ARG ODEION_PROJECT_GOOGLE_CLIENT_ID
# TIDAL's built-in clients (masked by build.rs): BuildKit secrets, read in this one RUN only, so they
# are in no layer, no build argument and not in the final image. Without them (a plain
# `docker build`) the build has none. Locally:
#   docker buildx build --no-cache-filter build \
#     --secret id=PM_TIDAL_PKCE_CLIENT_ID,env=PM_TIDAL_PKCE_CLIENT_ID … -t pubky-music-node .
RUN --mount=type=secret,id=PM_TIDAL_PKCE_CLIENT_ID \
    --mount=type=secret,id=PM_TIDAL_PKCE_CLIENT_SECRET \
    --mount=type=secret,id=PM_TIDAL_DEVICE_CLIENT_ID \
    --mount=type=secret,id=PM_TIDAL_DEVICE_CLIENT_SECRET \
    set -eu; \
    for k in PM_TIDAL_PKCE_CLIENT_ID PM_TIDAL_PKCE_CLIENT_SECRET PM_TIDAL_DEVICE_CLIENT_ID PM_TIDAL_DEVICE_CLIENT_SECRET; do \
        if [ -s "/run/secrets/$k" ]; then export "$k=$(cat "/run/secrets/$k")"; fi; \
    done; \
    if [ "$TARGETARCH" = "arm64" ] && [ "$(dpkg --print-architecture)" != "arm64" ]; then \
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
               CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
               AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar \
               PKG_CONFIG_ALLOW_CROSS=1 \
               PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig; \
        cargo build --release -p pm-node --features alsa --target aarch64-unknown-linux-gnu; \
        install -D target/aarch64-unknown-linux-gnu/release/odeiond /out/odeiond; \
    else \
        cargo build --release -p pm-node --features alsa; \
        install -D target/release/odeiond /out/odeiond; \
    fi

FROM debian:bookworm-slim
# ffmpeg makes the smaller copies phones download (/v1/library/tracks/{id}/download?format=…):
# Debian's 5.1 series (AAC, MP3 through LAME, Opus), pinned to it; security fixes within 5.1 still
# arrive. Bump it here when the base image moves on.
ARG FFMPEG_VERSION=7:5.1.*
RUN apt-get update && apt-get install -y --no-install-recommends libasound2 ca-certificates alsa-utils \
        "ffmpeg=${FFMPEG_VERSION}" \
    && ffmpeg -hide_banner -encoders | grep -qE ' aac ' && ffmpeg -hide_banner -encoders | grep -q libmp3lame \
    && ffmpeg -hide_banner -encoders | grep -q libopus \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 1000 -g audio -d /data pm
COPY --from=build /out/odeiond /usr/local/bin/odeiond
# compat: pre-Odeion name of the executable
RUN ln -s odeiond /usr/local/bin/pm-node
COPY deploy/docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh && mkdir -p /data /music && chown pm:audio /data
USER pm
VOLUME ["/data", "/music"]
EXPOSE 7788
ENV PM_LISTEN=0.0.0.0:7788 PM_DATA_DIR=/data PM_FOLDERS=/music RUST_LOG=info
LABEL org.opencontainers.image.title="Odeion node" \
      org.opencontainers.image.description="Odeion node (odeiond): your music library and zones, served by your own node"
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["serve"]
