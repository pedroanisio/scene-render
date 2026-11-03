# syntax=docker/dockerfile:1.19
#
# scene-render compiled from a git reference: a release tag, a branch or a commit.
#
#   docker build --build-arg SR_REF=v0.9.0 -t scene-render:0.9.0 .
#   docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/work" -v scene-render-cache:/cache \
#     scene-render:0.9.0 encode film.scene.xml
#
# The source comes from SR_GIT at SR_REF, not from the build context, so one Dockerfile builds every
# version, including tags that predate it. A tag must match the workspace version, the build runs the
# release smoke test, and a failing smoke test fails the build.

ARG SR_GIT=https://github.com/pedroanisio/scene-render.git
ARG SR_REF=main

FROM rust:1.99-bookworm AS build
ARG SR_GIT
ARG SR_REF
ADD --keep-git-dir=true ${SR_GIT}#${SR_REF} /src
WORKDIR /src
# the shipped binary carries the PDF resolve step (SREP 17), whose PDF library needs Rust 1.92 or later
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eu; \
    version=$(cargo pkgid --locked -p scene-render); \
    version=${version##*[#@]}; \
    case "$SR_REF" in v[0-9]*) test "$SR_REF" = "v$version" || { echo "tag $SR_REF is not version $version"; exit 1; } ;; esac; \
    cargo build --locked --release -p scene-render --features scene-render/pdf; \
    mkdir -p /out; \
    cp target/release/scene-render /out/; \
    git rev-parse HEAD > /out/COMMIT; \
    echo "$version" > /out/VERSION; \
    test "$(/out/scene-render --version)" = "scene-render $version"; \
    cp README.md CHANGELOG.md DISCLAIMER.md LICENSE NOTICE COMMERCIAL.md THIRD-PARTY-NOTICES.md capabilities.json /out/

FROM ubuntu:24.04
ARG SR_GIT
ARG SR_REF
# the runtime the CI renders with: Mesa's lavapipe (Vulkan) and EGL/GL drivers, FFmpeg, the fonts the goldens
# use, curl for destinations and heif-convert for HEIC stills
RUN set -eu; \
    apt-get update; \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates curl ffmpeg libegl1 libgl1-mesa-dri libvulkan1 mesa-vulkan-drivers \
      libheif-examples libheif-plugin-libde265 \
      fonts-dejavu-core fonts-dejavu-extra fonts-noto-color-emoji fonts-wqy-zenhei; \
    rm -rf /var/lib/apt/lists/*; \
    useradd --create-home --uid 10001 render; \
    install -d -o render -g render /work; \
    install -d -m 1777 /cache
COPY --from=build /out/scene-render /usr/local/bin/scene-render
COPY --from=build /out/ /usr/share/doc/scene-render/
RUN rm /usr/share/doc/scene-render/scene-render
# any uid can run the image (`--user "$(id -u):$(id -g)"` writes files the caller owns): the shader cache lives in
# /cache, which a volume can keep between runs, not in a home directory an arbitrary uid lacks
ENV XDG_CACHE_HOME=/cache \
    XDG_RUNTIME_DIR=/tmp
USER render
WORKDIR /work
RUN set -eu; \
    dir=$(mktemp -d); \
    printf '%s\n' '<scene version="1.2"><project width="64" height="36" fps="10" duration="0.2" background="#000000"/><composition><shape id="box" shape="rect" width="16" height="16" fill="#FFFFFF"/></composition></scene>' > "$dir/smoke.xml"; \
    scene-render validate "$dir/smoke.xml"; \
    scene-render render "$dir/smoke.xml" -o "$dir/smoke.png" --strict; \
    scene-render encode "$dir/smoke.xml" -o "$dir/smoke.mkv" --codec ffv1 --hw software --parallel 1 --strict --no-upload; \
    test "$(ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 "$dir/smoke.mkv")" = 2; \
    rm -rf "$dir" /cache/*
LABEL org.opencontainers.image.title="scene-render" \
      org.opencontainers.image.source="${SR_GIT}" \
      org.opencontainers.image.ref.name="${SR_REF}" \
      org.opencontainers.image.licenses="PolyForm-Noncommercial-1.0.0"
ENTRYPOINT ["scene-render"]
CMD ["--help"]
