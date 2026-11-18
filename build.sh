#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# Builds release binaries into ${OUT_DIR}/<arch>/gpu-exporter for each arch in TARGETARCHES ("host" builds only
# the host architecture).
set -euo pipefail

OUT_DIR="${OUT_DIR:-bin}"
TARGETARCHES="${TARGETARCHES:-amd64 arm64}"
export VERSION="${VERSION:-$(git describe --tags --always --dirty 2>/dev/null || echo dev)}"
export REVISION="${REVISION:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}"
export BRANCH="${BRANCH:-$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)}"
# Keep build machine paths out of the binary (`trim-paths` is not stable yet).
export RUSTFLAGS="--remap-path-prefix=${PWD}=. --remap-path-prefix=${CARGO_HOME:-${HOME}/.cargo}=cargo ${RUSTFLAGS:-}"

case "$(uname -m)" in
    x86_64|amd64) HOSTARCH=amd64 ;;
    aarch64|arm64) HOSTARCH=arm64 ;;
    *) echo "Unsupported host architecture: $(uname -m)" >&2; exit 1 ;;
esac

if [ "${TARGETARCHES}" = host ]; then
    TARGETARCHES="${HOSTARCH}"
fi

rm -rf "${OUT_DIR}"
for TARGETARCH in ${TARGETARCHES}; do
    case "${TARGETARCH}" in
        amd64) triple=x86_64-unknown-linux-gnu; gnu=x86_64-linux-gnu ;;
        arm64) triple=aarch64-unknown-linux-gnu; gnu=aarch64-linux-gnu ;;
        *) echo "Unsupported target architecture: ${TARGETARCH}" >&2; exit 1 ;;
    esac
    if [ "${TARGETARCH}" != "${HOSTARCH}" ]; then
        cc="${gnu}-gcc"
        if ! command -v "${cc}" >/dev/null 2>&1; then
            echo "Required C compiler not found for host=${HOSTARCH} target=${TARGETARCH}: ${cc}" >&2
            exit 1
        fi
        triple_env=$(tr 'a-z-' 'A-Z_' <<<"${triple}")
        export "CARGO_TARGET_${triple_env}_LINKER=${cc}" "CC_${triple//-/_}=${cc}"
    fi
    echo "Building for ${TARGETARCH} on ${HOSTARCH}..."
    cargo build --release --locked -p gpu-process-exporter --target "${triple}"
    install -D -m 755 "target/${triple}/release/gpu-exporter" "${OUT_DIR}/${TARGETARCH}/gpu-exporter"
done
