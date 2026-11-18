# Parity harness

Proves that a candidate exporter behaves like the Go reference (`7190ab9`). The contract is
[`CONTRACT.md`](CONTRACT.md).

```bash
make parity                                        # Rust against the recorded Go outputs
make parity PARITY_ARGS="--runs 5 --scenario 07"
parity/run.sh --reference go                       # Rust against a live Go build
parity/run.sh --reference go --candidate target/parity/gpu-exporter-go --record   # re-record goldens
```

`run.sh` needs cargo, curl, and openssl. It fetches etcd and kube-apiserver (unless `KUBEBUILDER_ASSETS` is set)
and builds the Rust exporter with `VERSION`, `REVISION`, and `BRANCH` (default `parity`). `--reference go` also
builds the Go reference from `REFERENCE_COMMIT`, which needs Go.

## Pieces

- `fake-nvml/`: `libnvidia-ml.so.1` serving frames from `$FAKE_NVML_DIR/scenario.json`. The harness writes the
  frame index to `frame`; the fake acks it in `ack.<pid>` on `nvmlDeviceGetCount_v2` and traces calls to
  `trace.<pid>`.
- `harness/`: starts etcd and kube-apiserver once (RBAC, static tokens), then per scenario a fake
  `HOST_PROC_MOUNT_POINT`, a node name, and both exporters on adjacent ports. Work files go to `target/parity/run/`.
- `harness/scenarios/*.yaml`: steps `start`, `kube` (create, status, patch, delete, restart_apiserver), `proc`,
  `frame`, `sleep`, `check`. Strings expand `${cid:x}`, `${puid:pod}`, `${now+3s}`, `${node}`, `${ns}`.
- `golden/`: Go outputs per scenario and check, without `go_*`, `promhttp_*`, and `process_*`.
- `harness/src/bin/live-compare.rs`: scrapes two running exporters every 20s and checks the live criteria
  (`LIVE-RESULTS.md`).
- `check-test-map.sh`: every Go test in `go-tests.txt` needs a Rust test of the same snake_case name, or an entry in
  `test-map-exceptions.txt`.

A check compares both `/metrics` outputs (or HTTP responses, or a stalled-header connection) until equal or its
timeout (10s), and also asserts its `non_empty` families, `absent` selectors, and hand-computed `values`. `rust_values` are
asserted only on a Rust candidate, for the families Rust deliberately fixes (`CONTRACT.md`).
