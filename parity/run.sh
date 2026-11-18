#!/usr/bin/env bash
# Runs the differential parity harness. Arguments are passed to the harness (--reference, --candidate, --runs,
# --scenario...). The Go reference is built only for --reference go, which needs a Go toolchain.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

ENVTEST_K8S_VERSION=1.36.2
declare -A ENVTEST_SHA512=(
  [amd64]=ea743186c8a799f5cf8faf16969f86189d003cb7d130e0ac4b58789f1e5748dcf30ebe91c837a10d5ac415383da3e10b9e64d65785c938c23e739781cfb76f08
  [arm64]=2d72ee985a8e262a3c57dc9f7f0fd891f6a8c7bf7ebaa2db6dc6d8eac8ae28181afe51c1f368b67756cdb40b10de9b205609e1726e5f27f7c6d824dd9c6649ac
)

# Fetches etcd and kube-apiserver into the setup-envtest layout and prints their directory.
envtest_assets() {
  local arch dir archive
  case "$(uname -m)" in
    x86_64|amd64) arch=amd64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) echo "Unsupported host architecture: $(uname -m)" >&2; return 1 ;;
  esac
  dir="${ENVTEST_BIN_DIR:-$HOME/.cache/envtest}/k8s/${ENVTEST_K8S_VERSION}-linux-${arch}"
  if [ ! -x "${dir}/kube-apiserver" ]; then
    archive=$(mktemp)
    curl -fsSLo "${archive}" "https://github.com/kubernetes-sigs/controller-tools/releases/download/envtest-v${ENVTEST_K8S_VERSION}/envtest-v${ENVTEST_K8S_VERSION}-linux-${arch}.tar.gz"
    echo "${ENVTEST_SHA512[${arch}]}  ${archive}" | sha512sum -c - >&2
    mkdir -p "${dir}"
    tar -xzf "${archive}" -C "${dir}" --strip-components=2
    rm "${archive}"
  fi
  echo "${dir}"
}

export VERSION=${VERSION:-parity} REVISION=${REVISION:-parity} BRANCH=${BRANCH:-parity}
KUBEBUILDER_ASSETS=${KUBEBUILDER_ASSETS:-$(envtest_assets)}
export KUBEBUILDER_ASSETS
cargo build -p fake-nvml -p parity-harness -p gpu-process-exporter
if [[ " $* " == *" --reference go "* || " $* " == *" --reference=go "* ]]; then
  parity/build-reference.sh
fi
exec target/debug/parity-harness "$@"
