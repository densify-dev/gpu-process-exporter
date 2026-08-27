#!/usr/bin/env python3
"""Deterministic synthetic GPU metrics for the local Grafana demo."""

from __future__ import annotations

import math
import os
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

PORT = int(os.environ.get("PORT", "9494"))
START = time.monotonic()
GPU_MEMORY_TOTAL_BYTES = 24 * 1024**3

GPU_1 = {"uuid": "GPU-demo-01", "model": "NVIDIA A10G"}
GPU_2 = {"uuid": "GPU-demo-02", "model": "NVIDIA A10G"}

WORKLOADS = (
    {
        "node": "gpu-node-01",
        "namespace": "ml-platform",
        "pod": "image-inference-7b6c8f",
        "container": "inference",
        "container_id": "containerd://demo-inference",
        "gpu_allocation_type": "exclusive",
        "requests": 1,
        "devices": (
            {**GPU_1, "memory_base_gib": 7.0, "memory_swing_gib": 1.2, "phase": 0.2, "sm": (45, 13), "memory_util": (38, 12)},
        ),
    },
    {
        "node": "gpu-node-01",
        "namespace": "ml-platform",
        "pod": "training-worker-5bd78f",
        "container": "trainer",
        "container_id": "containerd://demo-trainer",
        "gpu_allocation_type": "exclusive",
        "requests": 2,
        "devices": (
            {**GPU_1, "memory_base_gib": 5.0, "memory_swing_gib": 0.9, "phase": 1.7, "sm": (30, 10), "memory_util": (31, 10)},
            {**GPU_2, "memory_base_gib": 8.5, "memory_swing_gib": 1.3, "phase": 2.4, "sm": (50, 13), "memory_util": (52, 13)},
        ),
    },
    {
        "node": "gpu-node-01",
        "namespace": "batch-jobs",
        "pod": "embedding-batch-2c4d1a",
        "container": "worker",
        "container_id": "containerd://demo-embedding",
        "gpu_allocation_type": "shared",
        "requests": 1,
        "devices": (
            {**GPU_2, "memory_base_gib": 6.5, "memory_swing_gib": 1.1, "phase": 3.1, "sm": (24, 9), "memory_util": (24, 9)},
        ),
    },
)

METRIC_TYPES = {
    "kubex_gpu_container_requests": ("gauge", "GPU requests or fractions assigned to a container."),
    "kubex_gpu_container_memory_total_bytes": ("gauge", "Total memory of the GPU assigned to a container."),
    "kubex_gpu_container_memory_bytes": ("gauge", "GPU memory used by a container."),
    "kubex_gpu_container_memory_footprint_percent": ("gauge", "Container GPU memory use as a percentage of total GPU memory."),
    "kubex_gpu_container_sm_utilization_percent_seconds_total": ("counter", "Accumulated SM utilization in percentage-seconds."),
    "kubex_gpu_container_memory_utilization_percent_seconds_total": ("counter", "Accumulated memory utilization in percentage-seconds."),
}


def escape_label(value: object) -> str:
    return str(value).replace("\\", "\\\\").replace("\n", "\\n").replace('"', '\\"')


def format_labels(labels: dict[str, object]) -> str:
    return "{" + ",".join(f'{key}="{escape_label(value)}"' for key, value in labels.items()) + "}"


def sample(name: str, labels: dict[str, object], value: float) -> str:
    return f"{name}{format_labels(labels)} {value:.6f}"


def counter_value(elapsed: float, base: float, amplitude: float, phase: float) -> float:
    """Integrate a positive sinusoidal utilization signal from process start."""
    angular_frequency = 2 * math.pi / 90
    return base * elapsed - amplitude / angular_frequency * (
        math.cos(angular_frequency * elapsed + phase) - math.cos(phase)
    )


def render_metrics(now: float | None = None) -> str:
    elapsed = max(0.0, (time.monotonic() if now is None else now) - START)
    lines = []
    for name, (metric_type, help_text) in METRIC_TYPES.items():
        lines.extend((f"# HELP {name} {help_text}", f"# TYPE {name} {metric_type}"))

    for workload in WORKLOADS:
        workload_labels = {
            "node": workload["node"],
            "namespace": workload["namespace"],
            "pod": workload["pod"],
            "container": workload["container"],
            "container_id": workload["container_id"],
            "gpu_allocation_type": workload["gpu_allocation_type"],
        }
        lines.append(sample("kubex_gpu_container_requests", workload_labels, workload["requests"]))

        for device in workload["devices"]:
            labels = {
                **workload_labels,
                "gpu_uuid": device["uuid"],
                "gpu_model": device["model"],
            }
            memory_gib = device["memory_base_gib"] + device["memory_swing_gib"] * math.sin(
                elapsed / 18 + device["phase"]
            )
            memory_bytes = memory_gib * 1024**3
            sm_base, sm_amplitude = device["sm"]
            memory_base, memory_amplitude = device["memory_util"]
            lines.extend(
                (
                    sample("kubex_gpu_container_memory_total_bytes", labels, GPU_MEMORY_TOTAL_BYTES),
                    sample("kubex_gpu_container_memory_bytes", labels, memory_bytes),
                    sample(
                        "kubex_gpu_container_memory_footprint_percent",
                        labels,
                        memory_bytes / GPU_MEMORY_TOTAL_BYTES * 100,
                    ),
                    sample(
                        "kubex_gpu_container_sm_utilization_percent_seconds_total",
                        labels,
                        counter_value(elapsed, sm_base, sm_amplitude, device["phase"]),
                    ),
                    sample(
                        "kubex_gpu_container_memory_utilization_percent_seconds_total",
                        labels,
                        counter_value(elapsed, memory_base, memory_amplitude, device["phase"] + 0.8),
                    ),
                )
            )

    return "\n".join(lines) + "\n"


class MetricsHandler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802
        if urlparse(self.path).path != "/metrics":
            self.send_error(HTTPStatus.NOT_FOUND)
            return

        body = render_metrics().encode("utf-8")
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        return


if __name__ == "__main__":
    server = ThreadingHTTPServer(("0.0.0.0", PORT), MetricsHandler)
    print(f"serving synthetic metrics on port {PORT}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
