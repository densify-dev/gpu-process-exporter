// SPDX-License-Identifier: Apache-2.0

//! Domain model: tracked GPU containers, their label sets, and per-cycle metrics.

use crate::value::fingerprint;
use jiff::Timestamp;
use std::collections::HashMap;
use std::fmt;

pub const MIB: u64 = 1024 * 1024;

pub type Pid = u32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuAllocationType {
    #[default]
    K8sResource,
    KaiScheduler,
}

impl GpuAllocationType {
    pub fn as_str(self) -> &'static str {
        match self {
            GpuAllocationType::K8sResource => "K8sResource",
            GpuAllocationType::KaiScheduler => "KaiScheduler",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GpuContainerKey {
    pub pod_uid: String,
    pub container_id: String,
}

impl fmt::Display for GpuContainerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.pod_uid, self.container_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalculatedValues {
    pub requests: f64,
    pub limits: f64,
    pub total_memory: f64,
}

/// The per-device label set of a container, built once from the container labels plus UUID and model.
#[derive(Clone, Debug, Default)]
pub struct DeviceModelFingerprint {
    pub model_name: String,
    pub fingerprint: u64,
    is_set: bool,
    pub per_device_labels: Vec<String>,
}

impl DeviceModelFingerprint {
    #[cfg(test)]
    pub fn new(model_name: &str) -> Self {
        DeviceModelFingerprint {
            model_name: model_name.into(),
            ..Default::default()
        }
    }

    pub fn conditionally_set(&mut self, uuid: &str, model_name: &str, container_labels: &[String]) {
        if self.model_name.is_empty() {
            self.model_name = model_name.into();
        }
        if !self.is_set && !container_labels.is_empty() {
            self.per_device_labels = container_labels.to_vec();
            self.per_device_labels
                .extend([uuid.to_string(), self.model_name.clone()]);
            self.fingerprint = fingerprint(&self.per_device_labels);
            self.is_set = true;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LabelFingerprint {
    pub labels: Vec<String>,
    pub fingerprint: u64,
}

#[derive(Clone, Debug, Default)]
pub struct GpuContainer {
    pub key: GpuContainerKey,
    pub node_name: String,
    pub namespace: String,
    pub pod_name: String,
    pub container_name: String,
    pub gpu_allocation_type: GpuAllocationType,
    pub gpu_request: Option<i64>,
    pub gpu_limit: Option<i64>,
    pub gpu_fraction: Option<f64>,
    pub gpu_memory: Option<u64>,
    pub gpu_num_devices: Option<u64>,
    pub finished_at: Option<Timestamp>,
    pub(crate) calculated_values: Option<CalculatedValues>,
    pub device_labels: HashMap<String, DeviceModelFingerprint>,
}

impl GpuContainer {
    /// Calculated values are cached on the first success.
    pub fn get_calculated_values(&mut self, dev_info: &DeviceInfo) -> Option<CalculatedValues> {
        if self.calculated_values.is_none() {
            match self.calculate(dev_info) {
                Ok(values) => self.calculated_values = Some(values),
                Err(e) => crate::log!(
                    "Error calculating GPU resources for container {}: {e}",
                    self.key
                ),
            }
        }
        self.calculated_values
    }

    pub fn reset_calculated_values(&mut self) {
        self.calculated_values = None;
    }

    pub fn reset_device_label_cache(&mut self) {
        for device in self.device_labels.values_mut() {
            device.fingerprint = 0;
            device.is_set = false;
            device.per_device_labels.clear();
        }
    }

    pub fn container_labels(&self) -> Vec<String> {
        vec![
            self.node_name.clone(),
            self.namespace.clone(),
            self.pod_name.clone(),
            self.container_name.clone(),
            self.key.container_id.clone(),
            self.gpu_allocation_type.as_str().into(),
        ]
    }

    #[cfg(test)]
    pub fn per_device_labels(&mut self, device_key: &str) -> Vec<String> {
        self.per_device_labels_fingerprint(device_key).labels
    }

    pub fn per_device_labels_fingerprint(&mut self, device_key: &str) -> LabelFingerprint {
        let container_labels = self.container_labels();
        match self.device_labels.get_mut(device_key) {
            Some(dmf) => {
                let model_name = dmf.model_name.clone();
                dmf.conditionally_set(device_key, &model_name, &container_labels);
                LabelFingerprint {
                    labels: dmf.per_device_labels.clone(),
                    fingerprint: dmf.fingerprint,
                }
            }
            None => LabelFingerprint {
                labels: container_labels,
                fingerprint: 0,
            },
        }
    }

    #[cfg(test)]
    pub fn all_per_device_labels(&mut self) -> Vec<Vec<String>> {
        self.all_per_device_label_fingerprints()
            .into_iter()
            .map(|lf| lf.labels)
            .collect()
    }

    pub fn all_per_device_label_fingerprints(&mut self) -> Vec<LabelFingerprint> {
        let keys: Vec<String> = self.device_labels.keys().cloned().collect();
        keys.iter()
            .map(|k| self.per_device_labels_fingerprint(k))
            .collect()
    }

    fn calculate(&self, dev_info: &DeviceInfo) -> Result<CalculatedValues, &'static str> {
        if dev_info.total_memory == 0 {
            return Err("device total memory is zero");
        }
        let device_total = dev_info.total_memory as f64;
        if let (Some(request), Some(limit)) = (self.gpu_request, self.gpu_limit) {
            return Ok(CalculatedValues {
                requests: request as f64,
                limits: limit as f64,
                total_memory: device_total,
            });
        }
        let (mut req_lim, total_memory) = if let Some(fraction) = self.gpu_fraction {
            (fraction, device_total * fraction)
        } else if let Some(memory) = self.gpu_memory {
            let total = memory.wrapping_mul(MIB) as f64;
            (total / device_total, total)
        } else {
            return Err("no KAI annotations to determine fractional GPU request/limit");
        };
        if let Some(num_devices) = self.gpu_num_devices {
            req_lim *= num_devices as f64;
        }
        Ok(CalculatedValues {
            requests: req_lim,
            limits: req_lim,
            total_memory,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceInfo {
    pub uuid: String,
    pub model_name: String,
    /// Reported by NVML in bytes.
    pub total_memory: u64,
    pub accounting_enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuContainerMetrics {
    pub memory_bytes: u64,
    pub memory_total_bytes: u64,
    pub protected_memory_bytes: u64,
    pub sm_util: f64,
    pub enc_util: f64,
    pub dec_util: f64,
    pub mem_util: f64,
    pub ofa_util: f64,
    pub jpg_util: f64,
    pub accounting_gpu_util: u32,
    pub accounting_mem_util: u32,
    pub accounting_max_memory_bytes: u64,
    pub accounting_time_us: u64,
    pub accounting_start_time_us: u64,
    pub accounting_is_running: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CONTAINER_ID: &str = "container-id";
    const TEST_GPU_UUID: &str = "gpu-a";
    const TEST_MODEL_NAME: &str = "model-a";

    fn dev(total_memory: u64) -> DeviceInfo {
        DeviceInfo {
            total_memory,
            ..Default::default()
        }
    }

    #[test]
    fn gpu_allocation_type_string() {
        assert_eq!(GpuAllocationType::K8sResource.as_str(), "K8sResource");
        assert_eq!(GpuAllocationType::KaiScheduler.as_str(), "KaiScheduler");
    }

    #[test]
    fn gpu_container_key_string() {
        let key = GpuContainerKey {
            pod_uid: "pod-uid".into(),
            container_id: TEST_CONTAINER_ID.into(),
        };
        assert_eq!(key.to_string(), "pod-uid/container-id");
    }

    #[test]
    fn gpu_container_label_helpers() {
        let mut gc = GpuContainer {
            key: GpuContainerKey {
                container_id: TEST_CONTAINER_ID.into(),
                ..Default::default()
            },
            node_name: "node-a".into(),
            namespace: "ns-a".into(),
            pod_name: "pod-a".into(),
            container_name: "ctr-a".into(),
            device_labels: [
                (
                    TEST_GPU_UUID.to_string(),
                    DeviceModelFingerprint::new(TEST_MODEL_NAME),
                ),
                ("gpu-b".to_string(), DeviceModelFingerprint::new("model-b")),
            ]
            .into(),
            ..Default::default()
        };
        let container_labels = gc.container_labels();
        let want: Vec<String> = [
            "node-a",
            "ns-a",
            "pod-a",
            "ctr-a",
            TEST_CONTAINER_ID,
            "K8sResource",
        ]
        .map(String::from)
        .into();
        assert_eq!(container_labels, want);

        let mut want_per_device = want.clone();
        want_per_device.extend([TEST_GPU_UUID.to_string(), TEST_MODEL_NAME.to_string()]);
        assert_eq!(gc.per_device_labels(TEST_GPU_UUID), want_per_device);

        let dmf = gc.device_labels.get_mut(TEST_GPU_UUID).unwrap();
        dmf.conditionally_set(TEST_GPU_UUID, TEST_MODEL_NAME, &container_labels);
        assert_eq!(dmf.per_device_labels, want_per_device);
        assert_eq!(dmf.fingerprint, fingerprint(&want_per_device));

        assert_eq!(gc.all_per_device_labels().len(), 2);
    }

    #[test]
    fn gpu_container_calculate_k8s_resource() {
        let gc = GpuContainer {
            gpu_request: Some(2),
            gpu_limit: Some(3),
            ..Default::default()
        };
        let got = gc.calculate(&dev(16 * MIB)).unwrap();
        assert_eq!(
            got,
            CalculatedValues {
                requests: 2.0,
                limits: 3.0,
                total_memory: (16 * MIB) as f64
            }
        );
    }

    #[test]
    fn gpu_container_calculate_kai_fraction() {
        let gc = GpuContainer {
            gpu_fraction: Some(0.25),
            gpu_num_devices: Some(2),
            ..Default::default()
        };
        let got = gc.calculate(&dev(16 * MIB)).unwrap();
        assert_eq!(
            got,
            CalculatedValues {
                requests: 0.5,
                limits: 0.5,
                total_memory: (4 * MIB) as f64
            }
        );
    }

    #[test]
    fn gpu_container_calculate_kai_memory() {
        let gc = GpuContainer {
            gpu_memory: Some(4096),
            gpu_num_devices: Some(2),
            ..Default::default()
        };
        let got = gc.calculate(&dev(16384 * MIB)).unwrap();
        assert_eq!(
            got,
            CalculatedValues {
                requests: 0.5,
                limits: 0.5,
                total_memory: (4096 * MIB) as f64
            }
        );
    }

    #[test]
    fn gpu_container_calculate_errors() {
        assert!(GpuContainer::default().calculate(&dev(0)).is_err());
        assert!(GpuContainer::default().calculate(&dev(1)).is_err());
    }

    #[test]
    fn get_calculated_values_caches_first_result() {
        let mut gc = GpuContainer {
            gpu_fraction: Some(0.5),
            ..Default::default()
        };
        let first = gc.get_calculated_values(&dev(8 * MIB)).unwrap();
        let second = gc.get_calculated_values(&dev(16 * MIB)).unwrap();
        assert_eq!(first, second);
        assert_eq!(second.total_memory, (4 * MIB) as f64);
        gc.reset_calculated_values();
        assert_eq!(
            gc.get_calculated_values(&dev(16 * MIB))
                .unwrap()
                .total_memory,
            (8 * MIB) as f64
        );
    }
}
