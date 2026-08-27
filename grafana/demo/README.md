# Local Grafana demo

This stack shows the GPU Process Exporter dashboard with synthetic metrics for three workloads on two GPUs. It needs Docker Compose v2, but no NVIDIA hardware or Kubernetes cluster.

## Start the demo

Run these commands from the repository root:

```bash
docker compose -f grafana/demo/compose.yaml up -d

curl -fsS http://localhost:9090/-/ready
```

Wait about two minutes for Prometheus to collect enough samples for the rate panels. The stack exposes these local-only services:

- Grafana at <http://localhost:3000>
- Prometheus at <http://localhost:9090>
- Synthetic metrics at <http://localhost:9494/metrics>

The dashboard is provisioned automatically. Open it at:

```text
http://localhost:3000/d/kubex-gpu-process/kubex-gpu-process-exporter?from=now-2m&to=now&refresh=5s&var-DS_PROMETHEUS=prometheus
```

Leave Node, Namespace, Pod, Container, and GPU UUID set to `All`. Grafana permits anonymous admin access in this local-only demo.

## Capture the screenshot

Use a 1600 by 1400 browser viewport, select kiosk mode, and capture the full dashboard page as `grafana/gpu-process-exporter-dashboard.png`.

## Stop the demo

```bash
docker compose -f grafana/demo/compose.yaml down -v
```
