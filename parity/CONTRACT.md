# Parity contract

The Rust exporter must match the Go exporter at `7190ab9` on everything below, as observed from outside the
process: `/metrics` content, HTTP behavior, and exit status. Log text, log format, and error messages are not
part of the contract.

## Configuration

- `NODE_NAME` is required. It is trimmed; empty means exit non-zero.
- Optional values that fail to parse fall back to their default (`EXPORTER_PORT=abc` gives 9494).
  Intervals use Go `time.ParseDuration` syntax (`1m30s`, `1.5h`, `500ms`, `µs`).
- Both intervals must be > 0, the scrape interval an exact multiple of the poll interval, and the port within
  1024–65535. A violation means exit non-zero.

## Startup and shutdown

- Order: config, kube client (kubeconfig, then in-cluster), SelfSubjectAccessReview for `get`/`list`/`watch` on
  pods, pod watch filtered by `spec.nodeName`, initial sync. Then HTTP and the NVML loop start concurrently.
- NVML init failure means exit non-zero. SIGINT/SIGTERM stops gracefully (HTTP shutdown within 5s, NVML
  shutdown) with exit 0.
- The first collection runs one poll interval after start. Ticks are skipped when collection is slow.

## HTTP

- Only `EXPORTER_ENDPOINT` serves metrics; other paths return 404.
- Text exposition format 0.0.4, also for the Prometheus 3 default `Accept` header. gzip when requested.
  Header read timeout 5s.

## Pod to container rules

- K8s resource path: a container or init container with a `nvidia.com/gpu` request or limit. Skipped in
  namespace `kai-resource-reservation` or when `run.ai/reserve_for_gpu_index` is set. Request and limit use
  apimachinery `Quantity.AsInt64`; `"500m"` gives no value, so no requests/limits series and memory total from
  the device.
- KAI path: any of `gpu-fraction`, `gpu-memory` (MiB), `gpu-fraction-num-devices`. The container is chosen by
  `gpu-fraction-container-name` (main containers first, then init containers only with `restartPolicy: Always`),
  else the first main container. KAI runs after K8s, so it wins for the same container ID.
- Container IDs come from status by container name with `runtime://` stripped. An empty ID means skip.
- Finish time: matching `terminated.finishedAt` (current or last state), else pod `deletionTimestamp`, else
  "now" if the phase is Succeeded or Failed, else not finished.
- A container no longer desired finishes only with terminal evidence. A deleted pod finishes all its containers,
  at "now" if nothing else applies. A relist that drops a pod counts as a delete.

## Metric semantics

- Finishing a container sets memory bytes, footprint, and protected memory to 0 for every known per-device
  label set (creating the protected-memory series if absent). Later collections do not write memory values
  for it again. Memory total and counters are not guarded.
- Series are removed TTL = 1.5 × scrape interval after `FinishedAt`, by a sweep every 0.5 × scrape interval.
  Removal deletes all container and per-device series and the container's PID cache.
- Requests/limits exist only when calculated: explicit request and limit; KAI fraction × num-devices; KAI
  memory / device total × num-devices. Memory total is device total × fraction, or gpu-memory MiB, or the
  device total.
- Footprint = 100 × memory / total; NaN and Inf pass through.
- Counters (SM, mem, enc, dec, OFA, JPG) are added only for increments > 0.
- `protected_memory_bytes` first appears on a positive value, then follows every write including zeros.
- Accounting gauges exist only if accounting was enabled when the device was first seen.
  `accounting_time_us` is set only when > 0.

## NVML collection

- Device info (model, total memory from `MemoryInfo_v2` then v1, accounting mode) is cached per UUID on first
  success. A device whose UUID, name, or total memory lookup fails is skipped for that cycle.
- Per-PID memory is the max of the compute and graphics lists.
- Utilization samples: drop timestamps ≤ the device's last seen; dedupe by (timestamp, PID) keeping the last;
  process in ascending timestamp order; weight = timestamp delta in seconds, the poll interval for the first
  group ever. Legacy and extended APIs keep separate last-seen values.
- Each PID is enriched once per device per cycle; accounting stats are fetched at most once per PID per device
  per cycle.
- PID mapping reads `/host/proc/<pid>/cgroup`: lines matching
  `pod<uid>.slice/(crio|cri-containerd)-<64 hex>.scope`, `_` in the UID becomes `-`, exactly one UID and one
  container ID. Cached per PID, validated by the inode of `/host/proc/<pid>`. Unmapped PIDs and PIDs without a
  tracked container are ignored. The cgroupfs driver is not supported.

## Metric surface

The 18 `kubex_gpu_container_*` families with the names, types, HELP strings, and label names and order of
`pkg/prometheus/prometheus.go`, plus `kubex_gpu_process_exporter_build_info`.

## Accepted deviations

- Build info labels are `version`, `revision`, `branch`, `rustversion`, `os`, `arch`.
- `go_*` and `promhttp_*` are dropped. The `process_*` set is the `prometheus` crate's (no
  `process_network_*`, no `process_virtual_memory_max_bytes`).
- `EXPORTER_ENDPOINT` must start with `/`; other values fail at startup.
- Float text formatting may differ (`5.8720256e+09` vs `5872025600`); values parse equal.

## Known Go behaviors preserved

Found while building the harness. Rust keeps them unless decided otherwise.

- Several PIDs of one container on one device: memory bytes, protected memory, and accounting values are
  last-write-wins in map iteration order, not summed. Only utilization counters sum. Scenarios keep those
  values equal across PIDs so the output is deterministic.
- A container on several devices caches its calculated values (requests, limits, memory total) from whichever
  device is reported first, in map order. Scenarios use equal device totals.
