#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail

rm -rf bin
TRIMPATH=$(realpath ..)

case "$(uname -m)" in
    x86_64|amd64)
        HOSTARCH=amd64
        ;;
    aarch64|arm64)
        HOSTARCH=arm64
        ;;
    *)
        echo "Unsupported host architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

cc_for_target() {
    local targetarch=$1

    if [ "${targetarch}" = "${HOSTARCH}" ]; then
        echo "gcc"
        return
    fi

    case "${targetarch}" in
        amd64)
            echo "x86_64-linux-gnu-gcc"
            ;;
        arm64)
            echo "aarch64-linux-gnu-gcc"
            ;;
        *)
            echo "Unsupported target architecture: ${targetarch}" >&2
            return 1
            ;;
    esac
}

for TARGETARCH in amd64 arm64; do
    mkdir -p bin/${TARGETARCH}

    CC=$(cc_for_target "${TARGETARCH}")
    if ! command -v "${CC}" >/dev/null 2>&1; then
        echo "Required C compiler not found for host=${HOSTARCH} target=${TARGETARCH}: ${CC}" >&2
        exit 1
    fi

    echo "Building for ${TARGETARCH} on ${HOSTARCH} using CC=${CC}..."

    GOOS=linux \
    GOARCH=${TARGETARCH} \
    CGO_ENABLED=1 \
    CGO_CFLAGS="-Wno-deprecated-declarations" \
    CC=${CC} \
    go build -trimpath \
    -gcflags=-trimpath="${TRIMPATH}" \
    -asmflags=-trimpath="${TRIMPATH}" \
    -ldflags="-w -s" \
    -o bin/${TARGETARCH}/gpu-exporter cmd/main.go
done
