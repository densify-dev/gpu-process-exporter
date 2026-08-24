# SPDX-License-Identifier: Apache-2.0

ARG TARGETIMAGE=debian:trixie-slim
ARG BUILDIMAGE=golang:1.27.0-trixie

FROM --platform=${BUILDPLATFORM} ${BUILDIMAGE} AS build

ARG BUILDARCH
ARG TARGETARCH

RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends \
        gcc \
        g++ \
        gcc-aarch64-linux-gnu \
        gcc-x86-64-linux-gnu \
        libc6-dev-arm64-cross \
        libc6-dev-amd64-cross \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src/gpu-process-exporter
COPY . .

RUN set -eux; \
    buildarch="${BUILDARCH:-$(dpkg --print-architecture)}"; \
    case "${buildarch}" in \
        x86_64|amd64) buildarch=amd64 ;; \
        aarch64|arm64) buildarch=arm64 ;; \
        *) echo "unsupported BUILDARCH=${buildarch}" >&2; exit 1 ;; \
    esac; \
    if [ "${TARGETARCH}" = "${buildarch}" ]; then \
        case "${TARGETARCH}" in \
            amd64|arm64) cc=gcc ;; \
            *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
        esac; \
    else \
        case "${TARGETARCH}" in \
            amd64) cc=x86_64-linux-gnu-gcc ;; \
            arm64) cc=aarch64-linux-gnu-gcc ;; \
            *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
        esac; \
    fi; \
    command -v "${cc}" >/dev/null; \
    mkdir -p "/out/${TARGETARCH}"; \
    TRIMPATH="$(realpath ..)"; \
    GOOS=linux \
    GOARCH="${TARGETARCH}" \
    CGO_ENABLED=1 \
    CGO_CFLAGS="-Wno-deprecated-declarations" \
    CC="${cc}" \
    go build -trimpath \
        -gcflags=-trimpath="${TRIMPATH}" \
        -asmflags=-trimpath="${TRIMPATH}" \
        -ldflags="-w -s" \
        -o "/out/${TARGETARCH}/gpu-exporter" \
        cmd/main.go; \
    go run github.com/google/go-licenses/v2@v2.0.1 save \
        --ignore github.com/densify-dev/gpu-process-exporter \
        --save_path /out/third-party-licenses \
        ./cmd; \
    test -n "$(find /out/third-party-licenses -type f -print -quit)"

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

RUN apt-get update \
    && apt-get upgrade -y \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --chmod=755 --from=build /out/${TARGETARCH}/gpu-exporter /usr/local/bin/gpu-exporter
COPY --chmod=755 entrypoint.sh /usr/local/bin/entrypoint.sh
COPY --from=build /out/third-party-licenses/ /usr/share/licenses/gpu-process-exporter/third-party/
COPY --chmod=644 LICENSE NOTICE /usr/share/licenses/gpu-process-exporter/

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
