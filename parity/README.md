# Parity harness

Proves that a candidate exporter behaves like the Go reference (`7190ab9`). The contract is
[`CONTRACT.md`](CONTRACT.md).

```bash
make parity                                   # Go vs Go
make parity PARITY_CANDIDATE=<bin>            # Go vs candidate
make parity PARITY_CANDIDATE=<bin> PARITY_ARGS="--runs 5 --scenario 07"
parity/run.sh --reference golden --candidate <bin>   # against recorded Go outputs
parity/run.sh --candidate target/parity/gpu-exporter-go --record   # re-record goldens
```

`run.sh` needs Go, cargo, openssl, and `setup-envtest` on `PATH` (or `KUBEBUILDER_ASSETS`). It builds the
reference from `REFERENCE_COMMIT` with `VERSION`, `REVISION`, and `BRANCH` (default `parity`); build the candidate
with the same values so the build info labels compare.

## Pieces

- `fake-nvml/`: `libnvidia-ml.so.1` serving frames from `$FAKE_NVML_DIR/scenario.json`. The harness writes the
  frame index to `frame`; the fake acks it in `ack.<pid>` on `nvmlDeviceGetCount_v2` and traces calls to
  `trace.<pid>`.
- `harness/`: starts etcd and kube-apiserver once (RBAC, static tokens), then per scenario a fake
  `HOST_PROC_MOUNT_POINT`, a node name, and both exporters on adjacent ports. Work files go to `target/parity/run/`.
- `harness/scenarios/*.yaml`: steps `start`, `kube` (create, status, patch, delete, restart_apiserver), `proc`,
  `frame`, `sleep`, `check`. Strings expand `${cid:x}`, `${puid:pod}`, `${now+3s}`, `${node}`, `${ns}`.
- `golden/`: Go outputs per scenario and check, without `go_*`, `promhttp_*`, and `process_*`.

A check compares both `/metrics` outputs (or HTTP responses, or a stalled-header connection) until equal or its
timeout (10s), and also asserts its `non_empty` families, `absent` selectors, and hand-computed `values`.
