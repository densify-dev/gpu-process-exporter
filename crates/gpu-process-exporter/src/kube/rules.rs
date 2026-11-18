// SPDX-License-Identifier: Apache-2.0

//! Pure rules mapping a pod to the GPU containers it should track, and to their finish times.

use crate::model::{GpuAllocationType, GpuContainer, GpuContainerKey};
use crate::value::{MapProvider, Parse, find_parsed_value};
use jiff::Timestamp;
use k8s_openapi::api::core::v1::{
    Container, ContainerState, ContainerStateTerminated, ContainerStatus, Pod,
};
use std::collections::HashMap;

pub const NVIDIA_GPU_RESOURCE: &str = "nvidia.com/gpu";
pub const KAI_RESOURCE_RESERVATION_NAMESPACE: &str = "kai-resource-reservation";
pub const KAI_RESOURCE_RESERVATION_ANNOTATION: &str = "run.ai/reserve_for_gpu_index";
pub const KAI_FRACTION_ANNOTATION: &str = "gpu-fraction";
pub const KAI_MEMORY_ANNOTATION: &str = "gpu-memory";
pub const KAI_FRACTION_CONTAINER_ANNOTATION: &str = "gpu-fraction-container-name";
pub const KAI_NUM_DEVICES_ANNOTATION: &str = "gpu-fraction-num-devices";
const RESTART_POLICY_ALWAYS: &str = "Always";

/// Desired containers by container ID. KAI containers are added after K8s resource containers, so they win.
pub fn desired_gpu_containers_for_pod(pod: &Pod) -> HashMap<String, GpuContainer> {
    let mut desired = HashMap::new();
    add_k8s_resource_containers(pod, &mut desired);
    add_kai_containers(pod, &mut desired);
    desired
}

fn parse_annotation<T: Parse>(pod: &Pod, key: &str) -> Option<T> {
    find_parsed_value(&MapProvider(pod.metadata.annotations.as_ref()), key)
}

fn add_k8s_resource_containers(pod: &Pod, desired: &mut HashMap<String, GpuContainer>) {
    if pod.metadata.namespace.as_deref() == Some(KAI_RESOURCE_RESERVATION_NAMESPACE)
        || parse_annotation::<String>(pod, KAI_RESOURCE_RESERVATION_ANNOTATION).is_some()
    {
        return;
    }
    let Some(spec) = &pod.spec else { return };
    let containers = spec.containers.iter().map(|c| (c, false));
    let init_containers = spec.init_containers.iter().flatten().map(|c| (c, true));
    for (container, init) in containers.chain(init_containers) {
        let resources = container.resources.as_ref();
        let request = resources.and_then(|r| r.requests.as_ref()?.get(NVIDIA_GPU_RESOURCE));
        let limit = resources.and_then(|r| r.limits.as_ref()?.get(NVIDIA_GPU_RESOURCE));
        if request.is_none() && limit.is_none() {
            continue;
        }
        let Some(container_id) = container_id_for(pod, &container.name, init) else {
            continue;
        };
        let mut gc = new_gpu_container(
            pod,
            &container.name,
            &container_id,
            GpuAllocationType::K8sResource,
        );
        gc.gpu_request = request.and_then(|q| quantity_as_int64(&q.0));
        gc.gpu_limit = limit.and_then(|q| quantity_as_int64(&q.0));
        desired.insert(container_id, gc);
    }
}

fn add_kai_containers(pod: &Pod, desired: &mut HashMap<String, GpuContainer>) {
    let fraction = parse_annotation::<f64>(pod, KAI_FRACTION_ANNOTATION);
    let memory = parse_annotation::<u64>(pod, KAI_MEMORY_ANNOTATION);
    let num_devices = parse_annotation::<u64>(pod, KAI_NUM_DEVICES_ANNOTATION);
    if fraction.is_none() && memory.is_none() && num_devices.is_none() {
        return;
    }
    let Some((container, init)) = select_kai_container(pod) else {
        return;
    };
    let Some(container_id) = container_id_for(pod, &container.name, init) else {
        return;
    };
    let mut gc = new_gpu_container(
        pod,
        &container.name,
        &container_id,
        GpuAllocationType::KaiScheduler,
    );
    gc.gpu_fraction = fraction;
    gc.gpu_memory = memory;
    gc.gpu_num_devices = num_devices;
    desired.insert(container_id, gc);
}

/// The container named by `gpu-fraction-container-name` (main containers first, then init containers with
/// `restartPolicy: Always`), or else the first main container.
fn select_kai_container(pod: &Pod) -> Option<(&Container, bool)> {
    let spec = pod.spec.as_ref()?;
    let annotations = pod.metadata.annotations.as_ref();
    match annotations.and_then(|a| a.get(KAI_FRACTION_CONTAINER_ANNOTATION)) {
        Some(name) if !name.trim().is_empty() => {
            if let Some(c) = spec.containers.iter().find(|c| &c.name == name) {
                return Some((c, false));
            }
            let c = spec
                .init_containers
                .iter()
                .flatten()
                .find(|c| &c.name == name)?;
            (c.restart_policy.as_deref() == Some(RESTART_POLICY_ALWAYS)).then_some((c, true))
        }
        _ => spec.containers.first().map(|c| (c, false)),
    }
}

fn container_statuses(pod: &Pod, init: bool) -> &[ContainerStatus] {
    let status = pod.status.as_ref();
    let statuses = if init {
        status.and_then(|s| s.init_container_statuses.as_ref())
    } else {
        status.and_then(|s| s.container_statuses.as_ref())
    };
    statuses.map(Vec::as_slice).unwrap_or_default()
}

fn container_id_for(pod: &Pod, name: &str, init: bool) -> Option<String> {
    let status = container_statuses(pod, init)
        .iter()
        .find(|s| s.name == name)?;
    Some(normalize_container_id(status.container_id.as_deref()?)).filter(|id| !id.is_empty())
}

fn new_gpu_container(
    pod: &Pod,
    name: &str,
    container_id: &str,
    allocation: GpuAllocationType,
) -> GpuContainer {
    GpuContainer {
        key: GpuContainerKey {
            pod_uid: pod.metadata.uid.clone().unwrap_or_default(),
            container_id: container_id.into(),
        },
        node_name: pod
            .spec
            .as_ref()
            .and_then(|s| s.node_name.clone())
            .unwrap_or_default(),
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        pod_name: pod.metadata.name.clone().unwrap_or_default(),
        container_name: name.into(),
        gpu_allocation_type: allocation,
        ..Default::default()
    }
}

/// Strips the `<runtime>://` prefix.
pub fn normalize_container_id(container_id: &str) -> String {
    container_id
        .split_once("://")
        .map_or(container_id, |(_, id)| id)
        .to_string()
}

/// Copies the desired state into a tracked container, keeping its finish time and label caches.
pub fn update_gpu_container(dst: &mut GpuContainer, src: &GpuContainer) {
    let labels_changed = dst.node_name != src.node_name
        || dst.namespace != src.namespace
        || dst.pod_name != src.pod_name
        || dst.container_name != src.container_name;
    dst.node_name.clone_from(&src.node_name);
    dst.namespace.clone_from(&src.namespace);
    dst.pod_name.clone_from(&src.pod_name);
    dst.container_name.clone_from(&src.container_name);
    dst.gpu_allocation_type = src.gpu_allocation_type;
    dst.gpu_request = src.gpu_request;
    dst.gpu_limit = src.gpu_limit;
    dst.gpu_fraction = src.gpu_fraction;
    dst.gpu_memory = src.gpu_memory;
    dst.gpu_num_devices = src.gpu_num_devices;
    dst.reset_calculated_values();
    if labels_changed {
        dst.reset_device_label_cache();
    }
}

/// The observed finish time, else the pod deletion time, else now for a terminal pod phase.
pub fn finished_at_for_container(pod: &Pod, container: &GpuContainer) -> Option<Timestamp> {
    observed_finished_at_for_container(pod, container)
        .or_else(|| pod.metadata.deletion_timestamp.as_ref().map(|t| t.0))
        .or_else(|| {
            let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
            matches!(phase, Some("Failed" | "Succeeded")).then(Timestamp::now)
        })
}

/// The `terminated.finishedAt` (current or last state) of the status whose name and ID match the container.
pub fn observed_finished_at_for_container(
    pod: &Pod,
    container: &GpuContainer,
) -> Option<Timestamp> {
    fn matching<'a>(
        state: &'a Option<ContainerState>,
        container_id: &str,
    ) -> Option<&'a ContainerStateTerminated> {
        let terminated = state.as_ref()?.terminated.as_ref()?;
        let id = normalize_container_id(terminated.container_id.as_deref().unwrap_or_default());
        (id == container_id).then_some(terminated)
    }
    let id = container.key.container_id.as_str();
    [false, true]
        .into_iter()
        .flat_map(|init| container_statuses(pod, init))
        .filter(|s| s.name == container.container_name)
        // The current state wins over the last state, even without finishedAt (Go's zero time).
        .find_map(|s| {
            matching(&s.state, id)
                .or_else(|| matching(&s.last_state, id))?
                .finished_at
                .as_ref()
                .map(|t| t.0)
        })
}

/// Port of apimachinery `ParseQuantity` followed by `Quantity.AsInt64`: `None` when the string does not parse
/// or the value is not an exact int64 (fractions, `m`, `Pi`, more than 18 digits...).
pub fn quantity_as_int64(s: &str) -> Option<i64> {
    parse_quantity_int64(s).ok().flatten()
}

/// `Err` when apimachinery rejects the string, `Ok(None)` when it parses but has no int64 value.
fn parse_quantity_int64(s: &str) -> Result<Option<i64>, ()> {
    if s.is_empty() {
        return Err(());
    }
    if s == "0" {
        return Ok(Some(0));
    }
    let parts = parse_quantity_string(s)?;
    let (exponent, binary) = interpret_suffix(parts.suffix).ok_or(())?;
    let (num, denom) = (parts.num.len() as i32, parts.denom.len() as i32);
    let (mut scale, precision, mantissa) = if binary {
        if exponent >= 0 && denom == 0 {
            (
                0,
                15 - num - (exponent as f32 * 3.0 / 10.0) as i32 - 1,
                1i64 << exponent,
            )
        } else {
            (0, -1, 1)
        }
    } else {
        (exponent, 18 - (num + denom), 1)
    };
    if precision >= 0 {
        scale -= denom;
        if scale >= -9 {
            let value: i64 = format!("{}{}", parts.num, parts.denom)
                .parse()
                .map_err(|_| ())?;
            if let Some(result) = value.checked_mul(mantissa) {
                let result = if parts.positive { result } else { -result };
                return Ok(int64_amount_as_int64(result, scale));
            }
        }
    }
    // The inf.Dec slow path never yields an int64 value; it only fails when the value has no digits.
    if parts.value.bytes().any(|b| b.is_ascii_digit()) {
        Ok(None)
    } else {
        Err(())
    }
}

fn int64_amount_as_int64(value: i64, scale: i32) -> Option<i64> {
    match scale {
        0 => Some(value),
        s if s < 0 => None,
        s => (0..s).try_fold(value, |v, _| v.checked_mul(10)),
    }
}

struct QuantityParts<'a> {
    positive: bool,
    value: &'a str,
    num: &'a str,
    denom: &'a str,
    suffix: &'a str,
}

/// Port of apimachinery `parseQuantityString`.
fn parse_quantity_string(s: &str) -> Result<QuantityParts<'_>, ()> {
    let b = s.as_bytes();
    let end = b.len();
    let mut pos = 0;
    let mut positive = true;
    match b.first() {
        Some(b'-') => {
            positive = false;
            pos += 1;
        }
        Some(b'+') => pos += 1,
        _ => {}
    }
    let zeros = b[pos..].iter().take_while(|&&c| c == b'0').count();
    pos += zeros;
    if pos >= end {
        return Ok(QuantityParts {
            positive,
            value: "0",
            num: "0",
            denom: "",
            suffix: "",
        });
    }
    let digits = |from: usize| from + b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let num_end = digits(pos);
    let mut num = &s[pos..num_end];
    if num_end >= end {
        return Ok(QuantityParts {
            positive,
            value: s,
            num,
            denom: "",
            suffix: "",
        });
    }
    pos = num_end;
    if num.is_empty() {
        num = "0";
    }
    let mut denom = "";
    if b[pos] == b'.' {
        pos += 1;
        let denom_end = digits(pos);
        denom = &s[pos..denom_end];
        if denom_end >= end {
            return Ok(QuantityParts {
                positive,
                value: s,
                num,
                denom,
                suffix: "",
            });
        }
        pos = denom_end;
    }
    let value = &s[..pos];
    let suffix_start = pos;
    pos += b[pos..]
        .iter()
        .take_while(|c| b"eEinumkKMGTP".contains(c))
        .count();
    if pos < end && matches!(b[pos], b'-' | b'+') {
        pos += 1;
    }
    if digits(pos) < end {
        return Err(());
    }
    Ok(QuantityParts {
        positive,
        value,
        num,
        denom,
        suffix: &s[suffix_start..],
    })
}

/// Returns (exponent, binary): base 2 for binary suffixes, else base 10.
fn interpret_suffix(suffix: &str) -> Option<(i32, bool)> {
    let decimal = |e| Some((e, false));
    let binary = |e| Some((e, true));
    match suffix {
        "" => decimal(0),
        "n" => decimal(-9),
        "u" => decimal(-6),
        "m" => decimal(-3),
        "k" => decimal(3),
        "M" => decimal(6),
        "G" => decimal(9),
        "T" => decimal(12),
        "P" => decimal(15),
        "E" => decimal(18),
        "Ki" => binary(10),
        "Mi" => binary(20),
        "Gi" => binary(30),
        "Ti" => binary(40),
        "Pi" => binary(50),
        "Ei" => binary(60),
        s if s.len() > 1 && s.starts_with(['e', 'E']) => {
            let exponent: i64 = s[1..].parse().ok()?;
            decimal(exponent as i32)
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    pub(crate) const TEST_NODE_NAME: &str = "node-a";
    pub(crate) const TEST_NAMESPACE: &str = "test";
    pub(crate) const MAIN_CONTAINER_NAME: &str = "main";
    pub(crate) const KAI_INIT_CONTAINER_NAME: &str = "kai-init";

    pub(crate) fn pod(value: serde_json::Value) -> Pod {
        serde_json::from_value(value).unwrap()
    }

    pub(crate) fn running_status(name: &str, id: &str) -> serde_json::Value {
        json!({"name": name, "containerID": format!("containerd://{id}"), "image": "", "imageID": "", "ready": true,
               "restartCount": 0, "state": {"running": {}}})
    }

    pub(crate) fn terminated_status(name: &str, id: &str) -> serde_json::Value {
        let finished = Timestamp::now().to_string();
        json!({"name": name, "containerID": format!("containerd://{id}"), "image": "", "imageID": "", "ready": false,
               "restartCount": 0, "state": {"terminated": {"exitCode": 0, "containerID": format!("containerd://{id}"),
               "finishedAt": finished}}})
    }

    pub(crate) fn kai_fraction_pod(name: &str, container_id: &str, fraction: f64) -> Pod {
        pod(json!({
            "metadata": {"name": name, "namespace": TEST_NAMESPACE, "uid": format!("{name}-uid"),
                         "annotations": {KAI_FRACTION_ANNOTATION: fraction.to_string()}},
            "spec": {"nodeName": TEST_NODE_NAME, "containers": [{"name": MAIN_CONTAINER_NAME}]},
            "status": {"containerStatuses": [running_status(MAIN_CONTAINER_NAME, container_id)]},
        }))
    }

    pub(crate) fn irrelevant_pod(name: &str) -> Pod {
        pod(json!({
            "metadata": {"name": name, "namespace": TEST_NAMESPACE, "uid": format!("{name}-uid")},
            "spec": {"nodeName": TEST_NODE_NAME, "containers": [{"name": MAIN_CONTAINER_NAME}]},
            "status": {"containerStatuses": [running_status(MAIN_CONTAINER_NAME, &"9".repeat(64))]},
        }))
    }

    #[derive(Deserialize)]
    struct QuantityCase {
        #[serde(rename = "in")]
        input: String,
        int64: Option<i64>,
        #[serde(default)]
        error: bool,
    }

    #[test]
    fn quantity_as_int64_fixtures() {
        let cases: Vec<QuantityCase> =
            serde_json::from_str(include_str!("../../../../testdata/quantities.json")).unwrap();
        for c in cases {
            let got = parse_quantity_int64(&c.input);
            if c.error {
                assert!(got.is_err(), "parse({:?}) = {got:?}, want error", c.input);
            } else {
                assert_eq!(got, Ok(c.int64), "parse({:?})", c.input);
            }
        }
    }

    #[test]
    fn desired_gpu_containers_for_pod_tracks_kai_num_devices() {
        let mut p = irrelevant_pod("kai-num-devices");
        p.metadata.annotations =
            Some([(KAI_NUM_DEVICES_ANNOTATION.to_string(), "2".to_string())].into());
        let desired = desired_gpu_containers_for_pod(&p);
        assert_eq!(desired.len(), 1);
        let gc = desired.values().next().unwrap();
        assert_eq!(gc.gpu_allocation_type, GpuAllocationType::KaiScheduler);
        assert_eq!(gc.gpu_num_devices, Some(2));
    }

    #[test]
    fn desired_gpu_containers_for_pod_skips_kai_init_container_without_always_restart_policy() {
        let p = pod(json!({
            "metadata": {"name": "kai-init-skip", "namespace": TEST_NAMESPACE, "uid": "kai-init-skip-uid",
                         "annotations": {KAI_FRACTION_ANNOTATION: "0.5",
                                         KAI_FRACTION_CONTAINER_ANNOTATION: KAI_INIT_CONTAINER_NAME}},
            "spec": {"nodeName": TEST_NODE_NAME, "containers": [{"name": MAIN_CONTAINER_NAME}],
                     "initContainers": [{"name": KAI_INIT_CONTAINER_NAME, "restartPolicy": "OnFailure"}]},
            "status": {"containerStatuses": [running_status(MAIN_CONTAINER_NAME, &"ab".repeat(32))],
                       "initContainerStatuses": [running_status(KAI_INIT_CONTAINER_NAME, &"cd".repeat(32))]},
        }));
        assert!(desired_gpu_containers_for_pod(&p).is_empty());
    }

    #[test]
    fn update_gpu_container_copies_limit_and_num_devices() {
        let mut dst = GpuContainer {
            namespace: "old-ns".into(),
            pod_name: "old-pod".into(),
            container_name: "old-container".into(),
            gpu_request: Some(1),
            gpu_limit: Some(1),
            gpu_num_devices: Some(1),
            ..Default::default()
        };
        let src = GpuContainer {
            namespace: "new-ns".into(),
            pod_name: "new-pod".into(),
            container_name: "new-container".into(),
            gpu_request: Some(2),
            gpu_limit: Some(3),
            gpu_num_devices: Some(4),
            ..Default::default()
        };
        update_gpu_container(&mut dst, &src);
        assert_eq!(
            (dst.namespace.as_str(), dst.pod_name.as_str()),
            ("new-ns", "new-pod")
        );
        assert_eq!(dst.container_name, "new-container");
        assert_eq!(
            (dst.gpu_request, dst.gpu_limit, dst.gpu_num_devices),
            (Some(2), Some(3), Some(4))
        );
    }

    #[test]
    fn finish_rules() {
        let cid = "5".repeat(64);
        let mut p = kai_fraction_pod("finish", &cid, 0.5);
        let gc = desired_gpu_containers_for_pod(&p).remove(&cid).unwrap();
        assert_eq!(finished_at_for_container(&p, &gc), None);

        p.status.as_mut().unwrap().phase = Some("Succeeded".into());
        assert!(finished_at_for_container(&p, &gc).is_some());

        let deleted: Timestamp = "2026-01-02T03:04:05Z".parse().unwrap();
        p.metadata.deletion_timestamp = Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
            deleted,
        ));
        assert_eq!(finished_at_for_container(&p, &gc), Some(deleted));

        p.status.as_mut().unwrap().container_statuses = Some(vec![
            serde_json::from_value(terminated_status(MAIN_CONTAINER_NAME, &cid)).unwrap(),
        ]);
        let observed = observed_finished_at_for_container(&p, &gc).unwrap();
        assert_eq!(finished_at_for_container(&p, &gc), Some(observed));
        assert_ne!(observed, deleted);
    }

    #[test]
    fn normalize_container_id_strips_runtime() {
        assert_eq!(normalize_container_id("containerd://abc"), "abc");
        assert_eq!(normalize_container_id("abc"), "abc");
        assert_eq!(normalize_container_id(""), "");
    }
}
