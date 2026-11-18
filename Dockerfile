# SPDX-License-Identifier: Apache-2.0

ARG TARGETIMAGE=debian:trixie-slim
ARG BUILDIMAGE=rust:1.100.0-trixie

FROM --platform=${BUILDPLATFORM} ${BUILDIMAGE} AS build

ARG BUILDARCH
ARG TARGETARCH
ARG CARGO_ABOUT_VERSION=0.9.3

RUN export DEBIAN_FRONTEND=noninteractive; \
    echo "Installing packages: gcc-aarch64-linux-gnu gcc-x86-64-linux-gnu libc6-dev-arm64-cross libc6-dev-amd64-cross" \
    && apt-get update -qq >/dev/null \
    && apt-get upgrade -qq -y -o=Dpkg::Use-Pty=0 >/dev/null \
    && apt-get install -qq -y -o=Dpkg::Use-Pty=0 --no-install-recommends \
        gcc-aarch64-linux-gnu \
        gcc-x86-64-linux-gnu \
        libc6-dev-arm64-cross \
        libc6-dev-amd64-cross >/dev/null \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    case "${BUILDARCH:-$(dpkg --print-architecture)}" in \
        amd64) triple=x86_64-unknown-linux-musl; sha256=3ad1e94ea1e63d69d1961f4fb15ccbbe9575d906111f96d2cb4dce6df48f71b4 ;; \
        arm64) triple=aarch64-unknown-linux-musl; sha256=eb4d9ff6ebd2518152a8308729431a04b3836e71d2bd5ead7aea69f91a71162e ;; \
        *) echo "unsupported BUILDARCH=${BUILDARCH}" >&2; exit 1 ;; \
    esac; \
    archive="cargo-about-${CARGO_ABOUT_VERSION}-${triple}.tar.gz"; \
    curl -fsSLo "/tmp/${archive}" "https://github.com/EmbarkStudios/cargo-about/releases/download/${CARGO_ABOUT_VERSION}/${archive}"; \
    echo "${sha256}  /tmp/${archive}" | sha256sum -c -; \
    tar -xzf "/tmp/${archive}" -C /usr/local/cargo/bin --strip-components=1 --wildcards '*/cargo-about'; \
    rm "/tmp/${archive}"

WORKDIR /src/gpu-process-exporter
COPY . .

ARG VERSION=dev
ARG REVISION=unknown
ARG BRANCH=unknown

RUN set -eux; \
    OUT_DIR=/out TARGETARCHES="${TARGETARCH}" VERSION="${VERSION}" REVISION="${REVISION}" BRANCH="${BRANCH}" ./build.sh; \
    mkdir -p /out/third-party-licenses; \
    cargo about generate --locked -c about.toml -m crates/gpu-process-exporter/Cargo.toml \
        -o /out/third-party-licenses/THIRD-PARTY-LICENSES.txt about.hbs

FROM ${TARGETIMAGE}

ARG TARGETARCH
ARG VERSION=dev
ARG REVISION=unknown
ARG CREATED=unknown
ARG SOURCE_URL=https://github.com/densify-dev/gpu-process-exporter

LABEL org.opencontainers.image.title="GPU Process Exporter" \
      org.opencontainers.image.description="Prometheus exporter for Kubernetes container GPU process metrics" \
      org.opencontainers.image.vendor="Evenkeel Inc. d/b/a Kubex" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.source="${SOURCE_URL}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.created="${CREATED}"

RUN export DEBIAN_FRONTEND=noninteractive; \
    echo "Installing packages: ca-certificates" \
    && apt-get update -qq >/dev/null \
    && apt-get upgrade -qq -y -o=Dpkg::Use-Pty=0 >/dev/null \
    && apt-get install -qq -y -o=Dpkg::Use-Pty=0 --no-install-recommends ca-certificates >/dev/null \
    && rm -rf /var/lib/apt/lists/*

COPY --chmod=755 --from=build /out/${TARGETARCH}/gpu-exporter /usr/local/bin/gpu-exporter
COPY --chmod=755 entrypoint.sh /usr/local/bin/entrypoint.sh
COPY --from=build /out/third-party-licenses/ /usr/share/licenses/gpu-process-exporter/third-party/
COPY --chmod=644 LICENSE NOTICE /usr/share/licenses/gpu-process-exporter/

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
