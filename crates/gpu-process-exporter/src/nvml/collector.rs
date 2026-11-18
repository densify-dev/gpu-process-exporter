// SPDX-License-Identifier: Apache-2.0

//! Polls NVML and turns per-process GPU usage into per-container metrics.

use super::backend::{
    AccountingStats, GpuBackend, GpuDevice, NvmlBackend, ProcessDetail, ProcessInfo, UtilSample,
};
use crate::kube::store::Store;
use crate::log;
use crate::metrics::provider::{
    GpuMetricsProvider, add_value_conditional, positive_value, set_value, set_value_conditional,
};
use crate::model::{DeviceInfo, GpuContainerKey, GpuContainerMetrics};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const MICROSECONDS_IN_SECOND: f64 = 1_000_000.0;

type CurrentMetrics = HashMap<GpuContainerKey, HashMap<String, GpuContainerMetrics>>;

pub struct MetricsCollector {
    store: Arc<Store>,
    gmp: Arc<GpuMetricsProvider>,
    poll_interval: Duration,
    devices: HashMap<String, DeviceInfo>,
    utilization_last_seen_by_gpu: HashMap<String, u64>,
    extended_utilization_last_seen_by_gpu: HashMap<String, u64>,
    default_util_sample_interval: f64,
}

/// Per device and cycle: process details, accounting stats fetched so far, and PIDs already enriched.
struct DeviceCycle<'a> {
    device: &'a dyn GpuDevice,
    info: DeviceInfo,
    process_details: HashMap<u32, ProcessDetail>,
    accounting_stats: HashMap<u32, AccountingStats>,
    accounting_stats_known: HashSet<u32>,
    enriched_pids: HashSet<u32>,
}

impl DeviceCycle<'_> {
    /// Adds protected memory and accounting values, once per PID.
    fn enrich_once(&mut self, metrics: &mut GpuContainerMetrics, pid: u32) {
        if self.enriched_pids.insert(pid) {
            self.enrich(metrics, pid);
        }
    }

    fn enrich(&mut self, metrics: &mut GpuContainerMetrics, pid: u32) {
        if let Some(detail) = self.process_details.get(&pid) {
            metrics.protected_memory_bytes = detail.used_gpu_cc_protected_memory;
        }
        if !self.info.accounting_enabled {
            return;
        }
        let mut stats = self.accounting_stats.get(&pid).copied();
        if stats.is_none() && self.accounting_stats_known.insert(pid) {
            stats = self.device.accounting_stats(pid).ok();
            if let Some(s) = stats {
                self.accounting_stats.insert(pid, s);
            }
        }
        let Some(stats) = stats else { return };
        metrics.accounting_gpu_util = stats.gpu_utilization;
        metrics.accounting_mem_util = stats.memory_utilization;
        metrics.accounting_max_memory_bytes = stats.max_memory_usage;
        metrics.accounting_time_us = stats.time;
        metrics.accounting_start_time_us = stats.start_time;
        metrics.accounting_is_running = stats.is_running;
    }
}

impl MetricsCollector {
    pub fn new(store: Arc<Store>, poll_interval: Duration, gmp: Arc<GpuMetricsProvider>) -> Self {
        MetricsCollector {
            store,
            gmp,
            poll_interval,
            devices: HashMap::new(),
            utilization_last_seen_by_gpu: HashMap::new(),
            extended_utilization_last_seen_by_gpu: HashMap::new(),
            default_util_sample_interval: poll_interval.as_secs_f64(),
        }
    }

    /// Initializes NVML and collects every poll interval until cancelled. Slow collections skip ticks. NVML calls
    /// block, so this runs on a dedicated thread with its own single-threaded runtime for the ticker.
    pub fn run(mut self, cancel: CancellationToken) -> anyhow::Result<()> {
        let backend = NvmlBackend::init()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;
        runtime.block_on(async {
            let start = tokio::time::Instant::now() + self.poll_interval;
            let mut ticker = tokio::time::interval_at(start, self.poll_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = ticker.tick() => self.collect(&backend),
                    _ = cancel.cancelled() => break,
                }
            }
        });
        backend.shutdown()
    }

    fn collect(&mut self, backend: &dyn GpuBackend) {
        let device_count = match backend.device_count() {
            Ok(count) => count,
            Err(e) => {
                log!("nvml: get device count failed: {}", backend.error_string(e));
                return;
            }
        };
        let mut current = CurrentMetrics::new();
        for index in 0..device_count {
            let device = match backend.device(index) {
                Ok(device) => device,
                Err(e) => {
                    log!(
                        "nvml: get device handle for index {index} failed: {}",
                        backend.error_string(e)
                    );
                    continue;
                }
            };
            let uuid = match device.uuid() {
                Ok(uuid) if !uuid.is_empty() => uuid,
                result => {
                    let reason = result
                        .err()
                        .map(|e| backend.error_string(e))
                        .unwrap_or_default();
                    log!("nvml: get uuid for device index {index} failed: {reason}");
                    continue;
                }
            };
            let Some(info) = self.device_info(backend, device.as_ref(), &uuid) else {
                continue;
            };
            self.collect_device(device.as_ref(), &uuid, info, &mut current);
        }
        self.report_metrics(&current);
    }

    /// Device info, cached on first success.
    fn device_info(
        &mut self,
        backend: &dyn GpuBackend,
        device: &dyn GpuDevice,
        uuid: &str,
    ) -> Option<DeviceInfo> {
        if let Some(info) = self.devices.get(uuid) {
            return Some(info.clone());
        }
        let model_name = device
            .name()
            .inspect_err(|e| {
                log!(
                    "nvml: get model name for device {uuid} failed: {}",
                    backend.error_string(*e)
                )
            })
            .ok()?;
        let total_memory = [device.memory_total_v2(), device.memory_total()]
            .into_iter()
            .find_map(|total| total.ok().filter(|t| *t > 0));
        let Some(total_memory) = total_memory else {
            log!("nvml: get total memory for device {uuid} failed");
            return None;
        };
        let info = DeviceInfo {
            uuid: uuid.into(),
            model_name,
            total_memory,
            accounting_enabled: device.accounting_enabled().unwrap_or(false),
        };
        self.devices.insert(uuid.into(), info.clone());
        Some(info)
    }

    fn collect_device(
        &mut self,
        device: &dyn GpuDevice,
        uuid: &str,
        info: DeviceInfo,
        current: &mut CurrentMetrics,
    ) {
        let mut cycle = DeviceCycle {
            device,
            info,
            process_details: get_process_detail_map(device),
            accounting_stats: HashMap::new(),
            accounting_stats_known: HashSet::new(),
            enriched_pids: HashSet::new(),
        };
        for (pid, used_gpu_memory) in get_running_process_memory(device) {
            let Some(metrics) = self.metrics_for(current, uuid, pid) else {
                continue;
            };
            metrics.memory_bytes = used_gpu_memory;
            metrics.memory_total_bytes = cycle.info.total_memory;
            cycle.enrich_once(metrics, pid);
        }

        let last_seen = self
            .utilization_last_seen_by_gpu
            .get(uuid)
            .copied()
            .unwrap_or_default();
        if let Ok(samples) = device.process_utilization(last_seen) {
            let max_seen = self.add_utilization(
                &samples,
                last_seen,
                uuid,
                current,
                &mut cycle,
                |m, s, weight| {
                    m.sm_util += f64::from(s.sm) * weight;
                    m.mem_util += f64::from(s.mem) * weight;
                    m.enc_util += f64::from(s.enc) * weight;
                    m.dec_util += f64::from(s.dec) * weight;
                },
            );
            if max_seen > last_seen {
                self.utilization_last_seen_by_gpu
                    .insert(uuid.into(), max_seen);
            }
        }

        let last_seen = self
            .extended_utilization_last_seen_by_gpu
            .get(uuid)
            .copied()
            .unwrap_or_default();
        if let Ok(samples) = device.processes_utilization_info(last_seen) {
            let max_seen = self.add_utilization(
                &samples,
                last_seen,
                uuid,
                current,
                &mut cycle,
                |m, s, weight| {
                    m.ofa_util += f64::from(s.ofa) * weight;
                    m.jpg_util += f64::from(s.jpg) * weight;
                },
            );
            if max_seen > last_seen {
                self.extended_utilization_last_seen_by_gpu
                    .insert(uuid.into(), max_seen);
            }
        }
    }

    /// Adds weighted samples newer than `last_seen` and returns the newest timestamp seen.
    fn add_utilization(
        &self,
        samples: &[UtilSample],
        last_seen: u64,
        uuid: &str,
        current: &mut CurrentMetrics,
        cycle: &mut DeviceCycle<'_>,
        add: fn(&mut GpuContainerMetrics, &UtilSample, f64),
    ) -> u64 {
        let (groups, max_seen) = group_utilizations(samples, last_seen);
        let mut previous = last_seen;
        for (timestamp, by_pid) in groups {
            let weight = self.sample_weight_seconds(previous, timestamp);
            for (pid, sample) in by_pid {
                let Some(metrics) = self.metrics_for(current, uuid, pid) else {
                    continue;
                };
                add(metrics, &sample, weight);
                cycle.enrich_once(metrics, pid);
            }
            previous = timestamp;
        }
        max_seen
    }

    fn metrics_for<'a>(
        &self,
        current: &'a mut CurrentMetrics,
        uuid: &str,
        pid: u32,
    ) -> Option<&'a mut GpuContainerMetrics> {
        let key = self
            .store
            .mapper()
            .get_container_key(pid)
            .inspect_err(|e| log!("nvml: get container key for pid {pid}: {e}"))
            .ok()?;
        Some(
            current
                .entry(key)
                .or_default()
                .entry(uuid.into())
                .or_default(),
        )
    }

    fn sample_weight_seconds(&self, previous: u64, timestamp: u64) -> f64 {
        if previous == 0 {
            self.default_util_sample_interval
        } else if timestamp > previous {
            (timestamp - previous) as f64 / MICROSECONDS_IN_SECOND
        } else {
            0.0
        }
    }

    fn report_metrics(&self, current: &CurrentMetrics) {
        let gmp = &self.gmp;
        for (key, per_container) in current {
            for (uuid, m) in per_container {
                let Some(dev_info) = self.devices.get(uuid) else {
                    continue;
                };
                let Some(lv) = self.store.get_metric_labels_and_values(key, dev_info) else {
                    continue;
                };
                let labels = &lv.per_device_labels;
                let total_memory = match lv.calculated_values {
                    Some(cv) => {
                        set_value(&gmp.requests, &lv.container_labels, cv.requests);
                        set_value(&gmp.limits, &lv.container_labels, cv.limits);
                        cv.total_memory
                    }
                    None if m.memory_total_bytes == 0 => dev_info.total_memory as f64,
                    None => m.memory_total_bytes as f64,
                };
                set_value(&gmp.memory_total_bytes, labels, total_memory);
                let memory = m.memory_bytes as f64;
                gmp.set_memory_values(
                    labels,
                    lv.fingerprint,
                    memory,
                    m.protected_memory_bytes as f64,
                    100.0 * memory / total_memory,
                );
                for (counter, value) in [
                    (&gmp.sm_util, m.sm_util),
                    (&gmp.memory_util, m.mem_util),
                    (&gmp.enc_util, m.enc_util),
                    (&gmp.dec_util, m.dec_util),
                    (&gmp.ofa_util, m.ofa_util),
                    (&gmp.jpg_util, m.jpg_util),
                ] {
                    add_value_conditional(counter, labels, value, positive_value);
                }
                if dev_info.accounting_enabled {
                    set_value(
                        &gmp.accounting_gpu_util,
                        labels,
                        f64::from(m.accounting_gpu_util),
                    );
                    set_value(
                        &gmp.accounting_mem_util,
                        labels,
                        f64::from(m.accounting_mem_util),
                    );
                    set_value(
                        &gmp.accounting_max_memory_bytes,
                        labels,
                        m.accounting_max_memory_bytes as f64,
                    );
                    set_value_conditional(
                        &gmp.accounting_time_us,
                        labels,
                        m.accounting_time_us as f64,
                        positive_value,
                    );
                    set_value(
                        &gmp.accounting_start_time_us,
                        labels,
                        m.accounting_start_time_us as f64,
                    );
                    set_value(
                        &gmp.accounting_is_running,
                        labels,
                        f64::from(m.accounting_is_running),
                    );
                }
            }
        }
    }
}

/// Max memory per PID over the compute and graphics lists.
fn get_running_process_memory(device: &dyn GpuDevice) -> HashMap<u32, u64> {
    let mut memory = HashMap::new();
    let lists = [device.compute_processes(), device.graphics_processes()];
    for ProcessInfo {
        pid,
        used_gpu_memory,
    } in lists.into_iter().flatten().flatten()
    {
        let entry = memory.entry(pid).or_default();
        *entry = used_gpu_memory.max(*entry);
    }
    memory
}

fn get_process_detail_map(device: &dyn GpuDevice) -> HashMap<u32, ProcessDetail> {
    device
        .process_details()
        .unwrap_or_default()
        .into_iter()
        .map(|d| (d.pid, d))
        .collect()
}

/// Samples newer than `last_seen` grouped by ascending timestamp, the last sample winning per (timestamp, PID),
/// plus the newest timestamp seen.
fn group_utilizations(
    samples: &[UtilSample],
    last_seen: u64,
) -> (BTreeMap<u64, HashMap<u32, UtilSample>>, u64) {
    let mut groups: BTreeMap<u64, HashMap<u32, UtilSample>> = BTreeMap::new();
    for sample in samples.iter().filter(|s| s.timestamp > last_seen) {
        groups
            .entry(sample.timestamp)
            .or_default()
            .insert(sample.pid, *sample);
    }
    let max_seen = groups.keys().next_back().copied().unwrap_or(last_seen);
    (groups, max_seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::metrics::provider::tests::gauge_value;
    use crate::model::{GpuAllocationType, GpuContainer, MIB};
    use crate::nvml::backend::{NvmlError, NvmlResult};

    const ERROR_NOT_FOUND: NvmlError = NvmlError(6);
    use prometheus::Registry;
    use std::cell::Cell;

    const TEST_NODE_NAME: &str = "node-a";
    const TEST_NAMESPACE: &str = "ns-a";
    const TEST_POD_NAME: &str = "pod-a";
    const TEST_CONTAINER_NAME: &str = "ctr-a";
    const TEST_CONTAINER_ID: &str = "container-id-a";

    #[derive(Default)]
    struct MockDevice {
        compute: Vec<ProcessInfo>,
        graphics: Vec<ProcessInfo>,
        details: Vec<ProcessDetail>,
        stats: Option<AccountingStats>,
        stats_calls: Cell<usize>,
    }

    impl GpuDevice for MockDevice {
        fn uuid(&self) -> NvmlResult<String> {
            Err(ERROR_NOT_FOUND)
        }
        fn name(&self) -> NvmlResult<String> {
            Err(ERROR_NOT_FOUND)
        }
        fn memory_total_v2(&self) -> NvmlResult<u64> {
            Err(ERROR_NOT_FOUND)
        }
        fn memory_total(&self) -> NvmlResult<u64> {
            Err(ERROR_NOT_FOUND)
        }
        fn accounting_enabled(&self) -> NvmlResult<bool> {
            Err(ERROR_NOT_FOUND)
        }
        fn accounting_stats(&self, _pid: u32) -> NvmlResult<AccountingStats> {
            self.stats_calls.set(self.stats_calls.get() + 1);
            self.stats.ok_or(NvmlError(999))
        }
        fn compute_processes(&self) -> NvmlResult<Vec<ProcessInfo>> {
            Ok(self.compute.clone())
        }
        fn graphics_processes(&self) -> NvmlResult<Vec<ProcessInfo>> {
            Ok(self.graphics.clone())
        }
        fn process_utilization(&self, _last_seen: u64) -> NvmlResult<Vec<UtilSample>> {
            Ok(Vec::new())
        }
        fn processes_utilization_info(&self, _last_seen: u64) -> NvmlResult<Vec<UtilSample>> {
            Ok(Vec::new())
        }
        fn process_details(&self) -> NvmlResult<Vec<ProcessDetail>> {
            Ok(self.details.clone())
        }
    }

    fn cycle(device: &MockDevice, accounting_enabled: bool) -> DeviceCycle<'_> {
        DeviceCycle {
            device,
            info: DeviceInfo {
                accounting_enabled,
                ..Default::default()
            },
            process_details: super::get_process_detail_map(device),
            accounting_stats: HashMap::new(),
            accounting_stats_known: HashSet::new(),
            enriched_pids: HashSet::new(),
        }
    }

    fn collector(store: Store, gmp: Arc<GpuMetricsProvider>) -> MetricsCollector {
        MetricsCollector::new(Arc::new(store), Duration::from_millis(2500), gmp)
    }

    fn test_store(gmp: Arc<GpuMetricsProvider>) -> Store {
        let cfg = Config {
            node_name: TEST_NODE_NAME.into(),
            host_proc_mount_point: "/nonexistent".into(),
            driver_poll_interval: Duration::from_secs(1),
            scrape_interval: Duration::from_secs(1),
            exporter_endpoint: "/metrics".into(),
            exporter_port: 9494,
        };
        Store::new(&cfg, gmp)
    }

    fn group_process_utilization(ofa: bool) {
        let sample = |pid, timestamp, sm| UtilSample {
            pid,
            timestamp,
            sm,
            ofa: if ofa { sm } else { 0 },
            ..Default::default()
        };
        let samples = [
            sample(20, 200, 1),
            sample(20, 240, 2),
            sample(21, 230, 3),
            sample(22, 180, 4),
            sample(20, 240, 5),
        ];
        let (groups, max_seen) = group_utilizations(&samples, 200);
        assert_eq!(max_seen, 240);
        assert_eq!(groups.keys().copied().collect::<Vec<_>>(), [230, 240]);
        assert_eq!(groups[&230][&21].sm, 3);
        assert_eq!(
            groups[&240][&20].sm, 5,
            "duplicate timestamp keeps the last sample"
        );
        assert!(
            !groups[&230].contains_key(&22),
            "stale samples are filtered"
        );
    }

    #[test]
    fn group_process_utilization_info() {
        group_process_utilization(true);
    }

    #[test]
    fn group_process_utilization_samples() {
        group_process_utilization(false);
    }

    #[test]
    fn sample_weight_seconds_uses_default_for_first_sample() {
        let gmp = Arc::new(GpuMetricsProvider::new(&Registry::new()));
        let mc = collector(test_store(gmp.clone()), gmp);
        assert_eq!(mc.sample_weight_seconds(0, 40), 2.5);
        assert_eq!(mc.sample_weight_seconds(1_000_000, 3_500_000), 2.5);
        assert_eq!(mc.sample_weight_seconds(3_500_000, 3_500_000), 0.0);
    }

    #[test]
    fn enrich_gpu_metrics_sets_protected_memory_without_accounting() {
        let device = MockDevice {
            details: vec![ProcessDetail {
                pid: 7,
                used_gpu_cc_protected_memory: 64,
            }],
            ..Default::default()
        };
        let mut metrics = GpuContainerMetrics::default();
        cycle(&device, false).enrich(&mut metrics, 7);
        assert_eq!(metrics.protected_memory_bytes, 64);
        assert_eq!(metrics.accounting_gpu_util, 0);
        assert_eq!(device.stats_calls.get(), 0);
    }

    #[test]
    fn enrich_gpu_metrics_caches_accounting_stats() {
        let device = MockDevice {
            stats: Some(AccountingStats {
                gpu_utilization: 10,
                memory_utilization: 20,
                max_memory_usage: 30,
                time: 40,
                start_time: 50,
                is_running: 1,
            }),
            ..Default::default()
        };
        let mut metrics = GpuContainerMetrics::default();
        let mut cycle = cycle(&device, true);
        cycle.enrich(&mut metrics, 11);
        cycle.enrich(&mut metrics, 11);
        assert_eq!(device.stats_calls.get(), 1);
        assert_eq!(
            (metrics.accounting_gpu_util, metrics.accounting_mem_util),
            (10, 20)
        );
        assert_eq!(metrics.accounting_max_memory_bytes, 30);
        assert_eq!(
            (metrics.accounting_time_us, metrics.accounting_start_time_us),
            (40, 50)
        );
        assert_eq!(metrics.accounting_is_running, 1);
    }

    #[test]
    fn enrich_gpu_metrics_fetches_failed_accounting_stats_once() {
        let device = MockDevice::default();
        let mut cycle = cycle(&device, true);
        let mut metrics = GpuContainerMetrics::default();
        cycle.enrich(&mut metrics, 11);
        cycle.enrich(&mut metrics, 11);
        assert_eq!(device.stats_calls.get(), 1);
    }

    #[test]
    fn get_process_detail_map() {
        let device = MockDevice {
            details: vec![
                ProcessDetail {
                    pid: 21,
                    used_gpu_cc_protected_memory: 128,
                },
                ProcessDetail {
                    pid: 22,
                    used_gpu_cc_protected_memory: 256,
                },
            ],
            ..Default::default()
        };
        let got = super::get_process_detail_map(&device);
        assert_eq!(got.len(), 2);
        assert_eq!(got[&21].used_gpu_cc_protected_memory, 128);
        assert_eq!(got[&22].used_gpu_cc_protected_memory, 256);
    }

    #[test]
    fn get_running_process_memory_includes_graphics_only_processes() {
        let device = MockDevice {
            compute: vec![ProcessInfo {
                pid: 31,
                used_gpu_memory: 128,
            }],
            graphics: vec![ProcessInfo {
                pid: 32,
                used_gpu_memory: 256,
            }],
            ..Default::default()
        };
        assert_eq!(
            get_running_process_memory(&device),
            HashMap::from([(31, 128), (32, 256)])
        );
    }

    #[test]
    fn get_running_process_memory_deduplicates_compute_and_graphics_processes() {
        let device = MockDevice {
            compute: vec![ProcessInfo {
                pid: 41,
                used_gpu_memory: 128,
            }],
            graphics: vec![ProcessInfo {
                pid: 41,
                used_gpu_memory: 256,
            }],
            ..Default::default()
        };
        assert_eq!(
            get_running_process_memory(&device),
            HashMap::from([(41, 256)])
        );
    }

    #[test]
    fn report_metrics_sets_gauges_and_counters() {
        let uuid = "gpu-uuid-a";
        let key = GpuContainerKey {
            pod_uid: "pod-uid-a".into(),
            container_id: TEST_CONTAINER_ID.into(),
        };
        let gmp = Arc::new(GpuMetricsProvider::new(&Registry::new()));
        let store = test_store(gmp.clone());
        store.insert_for_test(GpuContainer {
            key: key.clone(),
            node_name: TEST_NODE_NAME.into(),
            namespace: TEST_NAMESPACE.into(),
            pod_name: TEST_POD_NAME.into(),
            container_name: TEST_CONTAINER_NAME.into(),
            gpu_request: Some(1),
            gpu_limit: Some(2),
            ..Default::default()
        });
        let mut mc = collector(store, gmp.clone());
        mc.devices.insert(
            uuid.into(),
            DeviceInfo {
                uuid: uuid.into(),
                model_name: "model-a".into(),
                total_memory: 2 * MIB,
                accounting_enabled: true,
            },
        );
        let metrics = GpuContainerMetrics {
            memory_bytes: 512,
            protected_memory_bytes: 8,
            sm_util: 4.0,
            mem_util: 5.0,
            enc_util: 6.0,
            dec_util: 7.0,
            ofa_util: 8.0,
            jpg_util: 9.0,
            accounting_gpu_util: 10,
            accounting_mem_util: 11,
            accounting_max_memory_bytes: 12,
            accounting_time_us: 13,
            accounting_start_time_us: 14,
            accounting_is_running: 1,
            ..Default::default()
        };
        mc.report_metrics(&HashMap::from([(
            key,
            HashMap::from([(uuid.to_string(), metrics)]),
        )]));

        let container_labels: Vec<String> = [
            TEST_NODE_NAME,
            TEST_NAMESPACE,
            TEST_POD_NAME,
            TEST_CONTAINER_NAME,
            TEST_CONTAINER_ID,
            GpuAllocationType::K8sResource.as_str(),
        ]
        .map(String::from)
        .into();
        let mut labels = container_labels.clone();
        labels.extend([uuid.to_string(), "model-a".to_string()]);
        assert_eq!(gauge_value(&gmp.requests, &container_labels), 1.0);
        assert_eq!(gauge_value(&gmp.limits, &container_labels), 2.0);
        let gauges = [
            (&gmp.memory_total_bytes, (2 * MIB) as f64),
            (&gmp.memory_bytes, 512.0),
            (&gmp.protected_memory_bytes, 8.0),
            (&gmp.accounting_gpu_util, 10.0),
            (&gmp.accounting_mem_util, 11.0),
            (&gmp.accounting_max_memory_bytes, 12.0),
            (&gmp.accounting_time_us, 13.0),
            (&gmp.accounting_start_time_us, 14.0),
            (&gmp.accounting_is_running, 1.0),
            (&gmp.memory_footprint, 100.0 * 512.0 / (2 * MIB) as f64),
        ];
        for (gauge, want) in gauges {
            assert_eq!(gauge_value(gauge, &labels), want);
        }
        let counters = [
            (&gmp.sm_util, 4.0),
            (&gmp.memory_util, 5.0),
            (&gmp.enc_util, 6.0),
            (&gmp.dec_util, 7.0),
            (&gmp.ofa_util, 8.0),
            (&gmp.jpg_util, 9.0),
        ];
        for (counter, want) in counters {
            assert_eq!(counter.with_label_values(&labels).get(), want);
        }
    }
}
