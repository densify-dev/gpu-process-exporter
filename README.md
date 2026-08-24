# GPU Process Exporter

GPU Process Exporter is a Kubex Prometheus exporter for Kubernetes nodes with NVIDIA GPUs. It reads per-process GPU data through NVIDIA Management Library, maps host PIDs back to Kubernetes pods and containers, and exposes container-labelled metrics.

The project is owned by Evenkeel Inc. d/b/a Kubex and licensed under Apache License 2.0. The first supported open-source release is `v1.1.0`. Earlier image tags are pre-open-source builds and are supported by Kubex but do not have their source open.

The canonical image is:

```text
docker.io/densify/gpu-process-exporter
```

## When to use it

Use this exporter when node-level or device-level GPU metrics are not enough and you need per-container attribution. It is designed for Linux Kubernetes nodes running NVIDIA GPUs and a container runtime that exposes host process and container metadata to privileged DaemonSet pods.

DCGM Exporter is still useful for device health and broad GPU telemetry. This exporter focuses on the process-to-container mapping path.

## Assumptions

- NVIDIA GPU nodes with NVML available on the host.
- Linux nodes.
- Kubernetes pods and containers visible through the host `/proc` tree.
- A deployment model that can mount host paths and run with the permissions needed to inspect host processes.
- Container IDs that can be resolved from cgroup data under the mounted host `/proc` tree.

## Configuration

| Variable | Default | Required | Description |
| --- | --- | --- | --- |
| `NODE_NAME` | none | yes | Kubernetes node name to use in metric labels and pod lookups. |
| `HOST_PROC_MOUNT_POINT` | `/host/proc` | no | Host `/proc` mount inside the exporter container. |
| `DRIVER_POLL_INTERVAL` | `1s` | no | How often the exporter polls NVML. |
| `SCRAPE_INTERVAL` | `60s` | no | Prometheus scrape interval. It must be a multiple of `DRIVER_POLL_INTERVAL`. |
| `EXPORTER_ENDPOINT` | `/metrics` | no | HTTP path for Prometheus metrics. |
| `EXPORTER_PORT` | `9494` | no | Metrics port. Must be between 1024 and 65535. |
| `NVML_SEARCH_PATH` | auto-detect | no | Host path containing `libnvidia-ml.so` if auto-detection does not find it. |

## Metrics

All metrics use Kubernetes container labels. Per-device metrics also include GPU UUID and model.

Container labels:

- `node`
- `namespace`
- `pod`
- `container`
- `container_id`
- `gpu_allocation_type`

Per-device labels add:

- `gpu_uuid`
- `gpu_model`

Exported metrics:

| Metric | Type | Description |
| --- | --- | --- |
| `kubex_gpu_container_requests` | gauge | Requested GPUs or GPU fractions. |
| `kubex_gpu_container_limits` | gauge | GPU limits or GPU fractions. |
| `kubex_gpu_container_memory_bytes` | gauge | GPU memory used by the container on a GPU. |
| `kubex_gpu_container_memory_total_bytes` | gauge | Total memory for the GPU. |
| `kubex_gpu_container_memory_footprint_percent` | gauge | Container memory use as a percent of GPU memory. |
| `kubex_gpu_container_sm_utilization_percent_seconds_total` | counter | Accumulated SM utilization. |
| `kubex_gpu_container_memory_utilization_percent_seconds_total` | counter | Accumulated memory utilization. |
| `kubex_gpu_container_enc_utilization_percent_seconds_total` | counter | Accumulated encoder utilization. |
| `kubex_gpu_container_dec_utilization_percent_seconds_total` | counter | Accumulated decoder utilization. |
| `kubex_gpu_container_ofa_utilization_percent_seconds_total` | counter | Accumulated OFA utilization. |
| `kubex_gpu_container_jpg_utilization_percent_seconds_total` | counter | Accumulated JPG utilization. |
| `kubex_gpu_container_protected_memory_bytes` | gauge | Protected memory reported by NVML. |
| `kubex_gpu_container_accounting_gpu_percent` | gauge | NVML accounting GPU utilization. |
| `kubex_gpu_container_accounting_memory_percent` | gauge | NVML accounting memory utilization. |
| `kubex_gpu_container_accounting_max_memory_bytes` | gauge | Maximum memory from NVML accounting. |
| `kubex_gpu_container_accounting_time_us` | gauge | Accounting runtime in microseconds. |
| `kubex_gpu_container_accounting_start_time_us` | gauge | Accounting start time in microseconds since epoch. |
| `kubex_gpu_container_accounting_is_running` | gauge | `1` if the process is still running, `0` otherwise. |

## Build and test

```bash
go test ./...
go test -race ./...
go build ./...
```

Build local binaries for amd64 and arm64:

```bash
./build.sh
```

Build a container image without pushing:

```bash
DOCKERHUB_TAG=v1.1.0 ./build-docker-image.sh
```

Push requires an explicit confirmation:

```bash
PUSH_IMAGE=true CONFIRM_PUSH=yes DOCKERHUB_TAG=v1.1.0 ./build-docker-image.sh
```

## Container runtime requirements

A Kubernetes deployment must mount at least:

- Host `/proc` at `/host/proc`, or set `HOST_PROC_MOUNT_POINT` to the mount path you use.
- Host root at `/host/root`, so `entrypoint.sh` can locate NVML libraries.

The pod needs permissions to read host process metadata and NVIDIA driver libraries. In practice this usually means a privileged DaemonSet or an equivalent security context with the required host mounts.

## Helm

No Helm chart is currently published for this exporter.

## Support

Support is best effort. Kubex does not provide an SLA or a backport promise for this open-source release.
