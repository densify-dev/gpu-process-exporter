# Live parity results

Recorded on 2026-11-18 on a two-node kind cluster (Kubernetes v1.36.1, containerd) with one NVIDIA T1000 8GB,
driver 615.71.09, serving a kubeai vLLM model. Go reference: 1.x at `7190ab9`. Rust: `feat/port-2-rust`.
`live-compare` scraped both exporters every 20s, skipped 2 warm-up scrapes, and checked counters over 5-minute
windows with a 1% tolerance.

## L3: side by side on the node

Go (port 9495) and Rust (port 9496) ran in one privileged `hostPID` pod with the production entrypoint.

| Check | Result |
| --- | --- |
| Duration | 90 min, 268 compared scrapes, 74 min since the last restart: PASS |
| Structure and gauge diffs | 0 |
| Counter windows | 17/17 within tolerance |
| Model pod deleted 3 times (the kubeai `Model` owns it) | Both followed the new container, no diffs |
| Accounting mode on, both restarted | Accounting gauges equal (float formatting aside) |
| Accounting mode off | Both dropped the gauges to 0; mode restored to `Disabled` |

Resources after 74 minutes:

| | RSS | CPU | Threads |
| --- | --- | --- | --- |
| Go | 63.2 MiB | 0.31% | 20 |
| Rust | 38.5 MiB | 0.44% | 27 |

Rust spends its CPU on one tokio worker. An instance with the extended utilization call disabled used the same
CPU, so it is not the OFA/JPG fix.

## L4: canary

The Helm release in `monitoring` ran image `2.0.0-canary` for 38 minutes, then was rolled back to `1.1.4`.

| Check | Result |
| --- | --- |
| `live-compare` against the Go shadow, 35 min | PASS: 103 compared scrapes, 0 diffs, 6/6 counter windows |
| Grafana dashboard queries | Every panel query returned data from the Rust exporter |
| `kubex_gpu_process_exporter_build_info` in Prometheus | New labels (`arch`, `os`, `rustversion`, `branch`) present |
| Rollback | Revision 9 manifest identical to revision 7 |

## Not covered live

- OFA and JPG counters: the T1000 reported no OFA or JPG samples, so Rust exposed none. The harness checks them
  with fake NVML.
- arm64 and multi-GPU nodes: covered by cross-compilation and harness scenarios only.
