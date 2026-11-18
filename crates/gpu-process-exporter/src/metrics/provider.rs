// SPDX-License-Identifier: Apache-2.0

//! The `kubex_gpu_container_*` metric families and the rules for writing them.

use crate::model::LabelFingerprint;
use prometheus::{CounterVec, GaugeVec, Opts, Registry};
use std::collections::HashSet;
use std::sync::Mutex;

pub const LABEL_NODE: &str = "node";
pub const LABEL_NAMESPACE: &str = "namespace";
pub const LABEL_POD: &str = "pod";
pub const LABEL_CONTAINER: &str = "container";
pub const LABEL_CONTAINER_ID: &str = "container_id";
pub const LABEL_GPU_ALLOCATION_TYPE: &str = "gpu_allocation_type";
pub const LABEL_GPU_UUID: &str = "gpu_uuid";
pub const LABEL_GPU_MODEL: &str = "gpu_model";

pub const CONTAINER_LABELS: [&str; 6] = [
    LABEL_NODE,
    LABEL_NAMESPACE,
    LABEL_POD,
    LABEL_CONTAINER,
    LABEL_CONTAINER_ID,
    LABEL_GPU_ALLOCATION_TYPE,
];
pub const PER_DEVICE_LABELS: [&str; 8] = [
    LABEL_NODE,
    LABEL_NAMESPACE,
    LABEL_POD,
    LABEL_CONTAINER,
    LABEL_CONTAINER_ID,
    LABEL_GPU_ALLOCATION_TYPE,
    LABEL_GPU_UUID,
    LABEL_GPU_MODEL,
];

const HELP_NUMBER_GPUS: &str = "Number or fraction of GPUs";
const HELP_BYTES: &str = "Bytes";
const HELP_PERCENT: &str = "Percent";
const HELP_UTILIZATION_CUMULATIVE: &str = "Accumulated utilization percent-seconds";
const HELP_MICROSECONDS: &str = "Microseconds";
const HELP_MICROSECONDS_SINCE_EPOCH: &str = "Microseconds since epoch";
const HELP_IS_RUNNING: &str = "1 if still running, 0 otherwise";

pub struct GpuMetricsProvider {
    pub requests: GaugeVec,
    pub limits: GaugeVec,
    pub memory_bytes: GaugeVec,
    pub memory_total_bytes: GaugeVec,
    pub memory_footprint: GaugeVec,
    pub sm_util: CounterVec,
    pub memory_util: CounterVec,
    pub enc_util: CounterVec,
    pub dec_util: CounterVec,
    pub ofa_util: CounterVec,
    pub jpg_util: CounterVec,
    pub protected_memory_bytes: GaugeVec,
    pub accounting_gpu_util: GaugeVec,
    pub accounting_mem_util: GaugeVec,
    pub accounting_max_memory_bytes: GaugeVec,
    pub accounting_time_us: GaugeVec,
    pub accounting_start_time_us: GaugeVec,
    pub accounting_is_running: GaugeVec,
    guards: Mutex<Guards>,
}

/// Fingerprints of per-device label sets: those whose protected memory series exists, and those of finished
/// containers whose memory values must not be written again.
#[derive(Default)]
struct Guards {
    protected_memory: HashSet<u64>,
    zeroed: HashSet<u64>,
}

impl GpuMetricsProvider {
    pub fn new(registry: &Registry) -> Self {
        let gauge = |name: &str, help: &str, labels: &[&str]| {
            let vec = GaugeVec::new(Opts::new(name, help), labels).unwrap();
            registry.register(Box::new(vec.clone())).unwrap();
            vec
        };
        let counter = |name: &str| {
            let vec = CounterVec::new(
                Opts::new(name, HELP_UTILIZATION_CUMULATIVE),
                &PER_DEVICE_LABELS,
            )
            .unwrap();
            registry.register(Box::new(vec.clone())).unwrap();
            vec
        };
        let per_device = |name: &str, help: &str| gauge(name, help, &PER_DEVICE_LABELS);
        GpuMetricsProvider {
            requests: gauge(
                "kubex_gpu_container_requests",
                HELP_NUMBER_GPUS,
                &CONTAINER_LABELS,
            ),
            limits: gauge(
                "kubex_gpu_container_limits",
                HELP_NUMBER_GPUS,
                &CONTAINER_LABELS,
            ),
            memory_bytes: per_device("kubex_gpu_container_memory_bytes", HELP_BYTES),
            memory_total_bytes: per_device("kubex_gpu_container_memory_total_bytes", HELP_BYTES),
            memory_footprint: per_device(
                "kubex_gpu_container_memory_footprint_percent",
                HELP_PERCENT,
            ),
            sm_util: counter("kubex_gpu_container_sm_utilization_percent_seconds_total"),
            memory_util: counter("kubex_gpu_container_memory_utilization_percent_seconds_total"),
            enc_util: counter("kubex_gpu_container_enc_utilization_percent_seconds_total"),
            dec_util: counter("kubex_gpu_container_dec_utilization_percent_seconds_total"),
            ofa_util: counter("kubex_gpu_container_ofa_utilization_percent_seconds_total"),
            jpg_util: counter("kubex_gpu_container_jpg_utilization_percent_seconds_total"),
            protected_memory_bytes: per_device(
                "kubex_gpu_container_protected_memory_bytes",
                HELP_BYTES,
            ),
            accounting_gpu_util: per_device(
                "kubex_gpu_container_accounting_gpu_percent",
                HELP_PERCENT,
            ),
            accounting_mem_util: per_device(
                "kubex_gpu_container_accounting_memory_percent",
                HELP_PERCENT,
            ),
            accounting_max_memory_bytes: per_device(
                "kubex_gpu_container_accounting_max_memory_bytes",
                HELP_BYTES,
            ),
            accounting_time_us: per_device(
                "kubex_gpu_container_accounting_time_us",
                HELP_MICROSECONDS,
            ),
            accounting_start_time_us: per_device(
                "kubex_gpu_container_accounting_start_time_us",
                HELP_MICROSECONDS_SINCE_EPOCH,
            ),
            accounting_is_running: per_device(
                "kubex_gpu_container_accounting_is_running",
                HELP_IS_RUNNING,
            ),
            guards: Mutex::default(),
        }
    }

    fn per_device_gauges(&self) -> [&GaugeVec; 10] {
        [
            &self.memory_bytes,
            &self.memory_total_bytes,
            &self.memory_footprint,
            &self.protected_memory_bytes,
            &self.accounting_gpu_util,
            &self.accounting_mem_util,
            &self.accounting_max_memory_bytes,
            &self.accounting_time_us,
            &self.accounting_start_time_us,
            &self.accounting_is_running,
        ]
    }

    fn counters(&self) -> [&CounterVec; 6] {
        [
            &self.sm_util,
            &self.memory_util,
            &self.enc_util,
            &self.dec_util,
            &self.ofa_util,
            &self.jpg_util,
        ]
    }

    /// Removes every series of the given containers and forgets their fingerprints.
    pub fn delete_values(&self, container_labels: &[Vec<String>], per_device: &[LabelFingerprint]) {
        for labels in container_labels {
            let _ = self.requests.remove_label_values(labels);
            let _ = self.limits.remove_label_values(labels);
        }
        let mut guards = self.guards.lock().unwrap();
        for lf in per_device {
            for gauge in self.per_device_gauges() {
                let _ = gauge.remove_label_values(&lf.labels);
            }
            for counter in self.counters() {
                let _ = counter.remove_label_values(&lf.labels);
            }
            guards.protected_memory.remove(&lf.fingerprint);
            guards.zeroed.remove(&lf.fingerprint);
        }
    }

    /// Sets the memory gauges of finished containers to 0 (creating them if absent) and guards them against later
    /// writes.
    pub fn zero_values(&self, per_device: &[LabelFingerprint]) {
        let mut guards = self.guards.lock().unwrap();
        for lf in per_device {
            guards.zeroed.insert(lf.fingerprint);
            for gauge in [
                &self.memory_bytes,
                &self.memory_footprint,
                &self.protected_memory_bytes,
            ] {
                set_value(gauge, &lf.labels, 0.0);
            }
        }
    }

    /// Writes the memory gauges unless the container was zeroed. Protected memory first appears on a positive
    /// value and then follows every write.
    pub fn set_memory_values(
        &self,
        labels: &[String],
        fingerprint: u64,
        memory: f64,
        protected: f64,
        footprint: f64,
    ) {
        let mut guards = self.guards.lock().unwrap();
        if guards.zeroed.contains(&fingerprint) {
            return;
        }
        set_value(&self.memory_bytes, labels, memory);
        if guards.protected_memory.contains(&fingerprint) || positive_value(protected) {
            guards.protected_memory.insert(fingerprint);
            set_value(&self.protected_memory_bytes, labels, protected);
        }
        set_value(&self.memory_footprint, labels, footprint);
    }
}

pub fn positive_value(value: f64) -> bool {
    value > 0.0
}

pub fn set_value(gauge: &GaugeVec, labels: &[String], value: f64) {
    gauge.with_label_values(labels).set(value);
}

pub fn set_value_conditional(
    gauge: &GaugeVec,
    labels: &[String],
    value: f64,
    cond: fn(f64) -> bool,
) -> bool {
    let ok = cond(value);
    if ok {
        set_value(gauge, labels, value);
    }
    ok
}

pub fn add_value_conditional(
    counter: &CounterVec,
    labels: &[String],
    value: f64,
    cond: fn(f64) -> bool,
) -> bool {
    let ok = cond(value);
    if ok {
        counter.with_label_values(labels).inc_by(value);
    }
    ok
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::GpuAllocationType;
    use crate::value::fingerprint;

    pub(crate) fn test_labels() -> Vec<String> {
        [
            "node-a",
            "ns-a",
            "pod-a",
            "ctr-a",
            "cid-a",
            GpuAllocationType::K8sResource.as_str(),
            "gpu-a",
            "model-a",
        ]
        .map(String::from)
        .into()
    }

    pub(crate) fn gauge_value(gauge: &GaugeVec, labels: &[String]) -> f64 {
        gauge.with_label_values(labels).get()
    }

    /// True when the gauge exposes a series with exactly these per-device label values.
    pub(crate) fn has_series(gauge: &GaugeVec, labels: &[String]) -> bool {
        let mut want: Vec<(&str, &str)> = PER_DEVICE_LABELS
            .iter()
            .copied()
            .zip(labels.iter().map(String::as_str))
            .collect();
        want.sort();
        gauge.collect_series().iter().any(|m| {
            m.get_label()
                .iter()
                .map(|l| (l.name(), l.value()))
                .eq(want.iter().copied())
        })
    }

    fn provider() -> GpuMetricsProvider {
        GpuMetricsProvider::new(&Registry::new())
    }

    #[test]
    fn set_value_conditional_and_add_value_conditional() {
        let gmp = provider();
        let labels = &test_labels()[..6];
        assert!(!set_value_conditional(
            &gmp.requests,
            labels,
            0.0,
            positive_value
        ));
        assert!(gmp.requests.collect_series().is_empty());
        assert!(set_value_conditional(
            &gmp.requests,
            labels,
            3.0,
            positive_value
        ));
        assert_eq!(gauge_value(&gmp.requests, labels), 3.0);

        let labels = test_labels();
        assert!(!add_value_conditional(
            &gmp.sm_util,
            &labels,
            0.0,
            positive_value
        ));
        assert_eq!(gmp.sm_util.with_label_values(&labels).get(), 0.0);
        assert!(add_value_conditional(
            &gmp.sm_util,
            &labels,
            5.0,
            positive_value
        ));
        assert_eq!(gmp.sm_util.with_label_values(&labels).get(), 5.0);
    }

    #[test]
    fn set_value_conditional_unless_fingerprinted_sets_zero_after_first_positive() {
        let gmp = provider();
        let labels = test_labels();
        let fp = fingerprint(&labels);
        gmp.set_memory_values(&labels, fp, 1.0, 0.0, 1.0);
        assert!(
            !has_series(&gmp.protected_memory_bytes, &labels),
            "absent while 0"
        );
        gmp.set_memory_values(&labels, fp, 1.0, 16.0, 1.0);
        assert_eq!(gauge_value(&gmp.protected_memory_bytes, &labels), 16.0);
        gmp.set_memory_values(&labels, fp, 1.0, 0.0, 1.0);
        assert_eq!(gauge_value(&gmp.protected_memory_bytes, &labels), 0.0);
    }

    #[test]
    fn delete_values_clears_fingerprints_by_gauge() {
        let gmp = provider();
        let labels = test_labels();
        let fp = fingerprint(&labels);
        gmp.set_memory_values(&labels, fp, 1.0, 16.0, 1.0);
        gmp.zero_values(&[LabelFingerprint {
            labels: labels.clone(),
            fingerprint: fp,
        }]);

        gmp.delete_values(
            &[labels[..6].to_vec()],
            &[LabelFingerprint {
                labels: labels.clone(),
                fingerprint: fp,
            }],
        );

        let guards = gmp.guards.lock().unwrap();
        assert!(!guards.protected_memory.contains(&fp));
        assert!(!guards.zeroed.contains(&fp));
        drop(guards);
        assert!(gmp.memory_bytes.collect_series().is_empty());
        assert!(gmp.protected_memory_bytes.collect_series().is_empty());
    }

    #[test]
    fn zero_values_clears_current_per_device_gauges_only() {
        let gmp = provider();
        let labels = test_labels();
        let container_labels = &labels[..6];
        set_value(&gmp.requests, container_labels, 1.0);
        set_value(&gmp.limits, container_labels, 2.0);
        for (i, gauge) in gmp.per_device_gauges().into_iter().enumerate() {
            set_value(gauge, &labels, i as f64 + 3.0);
        }

        gmp.zero_values(&[LabelFingerprint {
            labels: labels.clone(),
            fingerprint: fingerprint(&labels),
        }]);

        for gauge in [
            &gmp.memory_bytes,
            &gmp.memory_footprint,
            &gmp.protected_memory_bytes,
        ] {
            assert_eq!(gauge_value(gauge, &labels), 0.0);
        }
        assert_eq!(gauge_value(&gmp.requests, container_labels), 1.0);
        assert_eq!(gauge_value(&gmp.limits, container_labels), 2.0);
        assert_eq!(gauge_value(&gmp.memory_total_bytes, &labels), 4.0);
    }

    #[test]
    fn set_memory_values_does_not_resurrect_zeroed_container() {
        let gmp = provider();
        let labels = test_labels();
        let fp = fingerprint(&labels);
        gmp.set_memory_values(&labels, fp, 512.0, 128.0, 25.0);
        gmp.zero_values(&[LabelFingerprint {
            labels: labels.clone(),
            fingerprint: fp,
        }]);
        gmp.set_memory_values(&labels, fp, 256.0, 64.0, 12.5);
        for gauge in [
            &gmp.memory_bytes,
            &gmp.memory_footprint,
            &gmp.protected_memory_bytes,
        ] {
            assert_eq!(gauge_value(gauge, &labels), 0.0);
        }
    }

    #[test]
    fn zero_values_creates_protected_memory_series() {
        let gmp = provider();
        let labels = test_labels();
        gmp.zero_values(&[LabelFingerprint {
            labels: labels.clone(),
            fingerprint: fingerprint(&labels),
        }]);
        assert!(has_series(&gmp.protected_memory_bytes, &labels));
    }

    trait CollectSeries {
        fn collect_series(&self) -> Vec<prometheus::proto::Metric>;
    }

    impl<T: prometheus::core::Collector> CollectSeries for T {
        fn collect_series(&self) -> Vec<prometheus::proto::Metric> {
            self.collect()
                .into_iter()
                .flat_map(|mf| mf.get_metric().to_vec())
                .collect()
        }
    }
}
