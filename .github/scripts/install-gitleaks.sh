#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# Installs a checksum-verified gitleaks into ~/.local/bin and adds it to the job PATH.
set -euo pipefail

version=8.30.1
sha256=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb
archive="gitleaks_${version}_linux_x64.tar.gz"
dir=$(mktemp -d)
curl -fsSLo "${dir}/${archive}" "https://github.com/gitleaks/gitleaks/releases/download/v${version}/${archive}"
echo "${sha256}  ${dir}/${archive}" | sha256sum -c -
mkdir -p "${HOME}/.local/bin"
tar -xzf "${dir}/${archive}" -C "${HOME}/.local/bin" gitleaks
echo "${HOME}/.local/bin" >> "${GITHUB_PATH}"
