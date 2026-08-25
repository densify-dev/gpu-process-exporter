#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail

TARGET_IMAGE="${TARGET_IMAGE:-debian:trixie-slim}"
BUILD_IMAGE="${BUILD_IMAGE:-golang:1.27.0-trixie}"
DOCKERHUB_REPO="${DOCKERHUB_REPO:-densify/gpu-process-exporter}"
DOCKERHUB_TAG="${DOCKERHUB_TAG:-}"
PUSH_IMAGE="${PUSH_IMAGE:-false}"
CONFIRM_PUSH="${CONFIRM_PUSH:-no}"
CI="${CI:-false}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
SOURCE_URL="${SOURCE_URL:-https://github.com/densify-dev/gpu-process-exporter}"
CREATED="${CREATED:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
REVISION="${REVISION:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}"
VERSION="${VERSION:-${DOCKERHUB_TAG:-dev}}"
if [ "${DOCKERHUB_TAG}" = "v" ]; then
    echo "Refusing empty Docker Hub tag after removing v prefix." >&2
    exit 1
fi
DOCKERHUB_TAG="${DOCKERHUB_TAG#v}"

case "${DOCKERHUB_TAG}" in
    0.2.0-beta2|0.2.0-beta3|1.0.0)
        echo "Refusing to rebuild historical tag ${DOCKERHUB_TAG}." >&2
        exit 1
        ;;
esac

setupBuildxLocal() {
    if ! docker buildx version >/dev/null 2>&1; then
        echo "Docker Buildx is not available. Please enable buildx." >&2
        exit 1
    fi
    if ! docker buildx inspect multiarch-builder >/dev/null 2>&1; then
        docker buildx create \
            --name multiarch-builder \
            --driver docker-container \
            --driver-opt network=host \
            --bootstrap \
            --use
    else
        docker buildx use multiarch-builder
    fi
}

setupBuildxCI() {
    docker buildx create --name multiarch-builder --driver docker-container --use
    docker buildx inspect --bootstrap
    docker run --rm --privileged tonistiigi/binfmt --install all
}

if [ "${CI}" = "true" ]; then
    setupBuildxCI
else
    setupBuildxLocal
fi

if [ -z "${DOCKERHUB_TAG}" ]; then
    short_revision="${REVISION}"
    short_revision="${short_revision:0:12}"
    DOCKERHUB_TAG="dev-${short_revision}"
fi

image="${DOCKERHUB_REPO}:${DOCKERHUB_TAG}"
buildArgs=(
    --platform "${PLATFORMS}"
    --build-arg "TARGETIMAGE=${TARGET_IMAGE}"
    --build-arg "BUILDIMAGE=${BUILD_IMAGE}"
    --build-arg "VERSION=${VERSION}"
    --build-arg "REVISION=${REVISION}"
    --build-arg "CREATED=${CREATED}"
    --build-arg "SOURCE_URL=${SOURCE_URL}"
    -t "${image}"
)

if [ "${PUSH_IMAGE}" = "true" ]; then
    if [ "${CONFIRM_PUSH}" != "yes" ]; then
        echo "Set CONFIRM_PUSH=yes to publish ${image}." >&2
        exit 1
    fi
    if [ "${DOCKERHUB_TAG}" = "latest" ]; then
        echo "Refusing to publish latest. Use an immutable version tag." >&2
        exit 1
    fi
    buildArgs+=(--push --provenance=mode=max --sbom=true)
else
    mkdir -p bin
    buildArgs+=(--output "type=oci,dest=bin/gpu-process-exporter-image.tar" --provenance=false)
fi

DOCKER_BUILDKIT=1 docker buildx build "${buildArgs[@]}" .
