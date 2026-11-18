#!/usr/bin/env bash
# Runs the differential parity harness. Arguments are passed to the harness (--candidate, --runs, --scenario...).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

export VERSION=${VERSION:-parity} REVISION=${REVISION:-parity} BRANCH=${BRANCH:-parity}
if [ -z "${KUBEBUILDER_ASSETS:-}" ]; then
  KUBEBUILDER_ASSETS=$(setup-envtest use -p path --bin-dir "${ENVTEST_BIN_DIR:-$HOME/.cache/envtest}" "${ENVTEST_K8S_VERSION:-1.36.2}")
  export KUBEBUILDER_ASSETS
fi
cargo build -p fake-nvml -p parity-harness -p gpu-process-exporter
parity/build-reference.sh
exec target/debug/parity-harness "$@"
