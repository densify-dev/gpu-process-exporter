#!/usr/bin/env bash
# Builds the Go parity reference from REFERENCE_COMMIT into target/parity/gpu-exporter-go.
set -euo pipefail

REFERENCE_COMMIT=${REFERENCE_COMMIT:-7190ab9}
cd "$(git rev-parse --show-toplevel)"
src=target/parity/go-src
rm -rf "$src"
mkdir -p "$src"
git archive "$REFERENCE_COMMIT" | tar -x -C "$src"
pkg=github.com/prometheus/common/version
cd "$src"
CGO_ENABLED=1 CGO_CFLAGS="-Wno-deprecated-declarations" go build -trimpath \
  -ldflags="-X $pkg.Version=${VERSION} -X $pkg.Revision=${REVISION} -X $pkg.Branch=${BRANCH}" \
  -o ../gpu-exporter-go ./cmd
