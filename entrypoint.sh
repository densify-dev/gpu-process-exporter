#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
set -e

ROOT_MOUNT_POINT="/host/root"
NVML_LIB="libnvidia-ml.so"
NVML_RUNTIME_PATH="/run/gpu-process-exporter/nvidia-libs"
CUSTOM_SEARCH_PATH=false
if [[ -n "${NVML_SEARCH_PATH}" ]]; then
    NVML_SEARCH_PATHS=("${ROOT_MOUNT_POINT}/${NVML_SEARCH_PATH#/}")
    echo "Using custom NVML search path: ${NVML_SEARCH_PATH}"
    CUSTOM_SEARCH_PATH=true
else
    case "$(uname -m)" in
        x86_64|amd64)
            DEBIAN_LIB_ARCH="x86_64-linux-gnu"
            ;;
        aarch64|arm64)
            DEBIAN_LIB_ARCH="aarch64-linux-gnu"
            ;;
        *)
            DEBIAN_LIB_ARCH="$(uname -m)-linux-gnu"
            echo "WARNING: unknown architecture $(uname -m); trying Debian multiarch path ${DEBIAN_LIB_ARCH}"
            ;;
    esac

    # Add known host paths mapped into the container via /host/root
    NVML_SEARCH_PATHS=(
        "${ROOT_MOUNT_POINT}/home/kubernetes/bin/nvidia/lib64"         # GKE COS / GKE GPU Operator with Google driver installer
        "${ROOT_MOUNT_POINT}/opt/nvidia/lib64"                         # GKE Ubuntu Google driver installer
        "${ROOT_MOUNT_POINT}/usr/local/nvidia/lib64"                   # NVIDIA container runtime / GKE exposed driver path / kind and nvkind
        "${ROOT_MOUNT_POINT}/run/nvidia/driver/usr/lib64"              # NVIDIA GPU Operator driver container, RPM-style
        "${ROOT_MOUNT_POINT}/run/nvidia/driver/usr/lib/${DEBIAN_LIB_ARCH}" # NVIDIA GPU Operator driver container, Debian-style
        "${ROOT_MOUNT_POINT}/usr/lib/${DEBIAN_LIB_ARCH}"               # Ubuntu/Debian (GKE Ubuntu, AKS Ubuntu, OKE Ubuntu, kind)
        "${ROOT_MOUNT_POINT}/usr/lib64"                                # EKS Amazon Linux, AKS Azure Linux, OKE Oracle Linux, Bottlerocket
        "${ROOT_MOUNT_POINT}/lib/${DEBIAN_LIB_ARCH}"                   # Debian/Ubuntu merged-/usr compatibility
        "${ROOT_MOUNT_POINT}/lib64"                                    # RPM-style compatibility
    )
    echo "Using well-known NVML search paths"
fi

mkdir -p "${NVML_RUNTIME_PATH}"

NVML_FOUND=false

for path in "${NVML_SEARCH_PATHS[@]}"; do
    # go-nvml usually looks for libnvidia-ml.so.1 specifically
    if [[ -f "${path}/${NVML_LIB}.1" || -f "${path}/${NVML_LIB}" ]]; then
        find "${NVML_RUNTIME_PATH}" -mindepth 1 -maxdepth 1 -exec rm -f {} +
        # Keep host system libraries out of the dynamic linker's search path.
        # Some fallback paths are full OS library directories; putting those
        # directly in LD_LIBRARY_PATH can make the exporter load host libc.
        find "${path}" -maxdepth 1 \( -type f -o -type l \) \( -name 'libnvidia*.so*' -o -name 'libcuda*.so*' \) -exec ln -sf {} "${NVML_RUNTIME_PATH}/" \;
        if [[ -n "${LD_LIBRARY_PATH}" ]]; then
            export LD_LIBRARY_PATH="${NVML_RUNTIME_PATH}:${LD_LIBRARY_PATH}"
        else
            export LD_LIBRARY_PATH="${NVML_RUNTIME_PATH}"
        fi
        echo "Successfully located NVIDIA ML library ${NVML_LIB} at: ${path}"
        NVML_FOUND=true
        break
    fi
done

if [[ "${NVML_FOUND}" == false ]]; then
    if [[ "${CUSTOM_SEARCH_PATH}" == false ]]; then
        echo "WARNING: ${NVML_LIB} not found in well-known host paths."
    else
        echo "ERROR: ${NVML_LIB} not found in custom search path: ${NVML_SEARCH_PATH}"
    fi
    echo "Make sure to configure environment variable NVML_SEARCH_PATH"
    echo "with the folder where ${NVML_LIB} resides on the node."
    exit 1
fi

# Hand over process control to the GPU exporter
exec /usr/local/bin/gpu-exporter "$@"
