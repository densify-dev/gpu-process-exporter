// SPDX-License-Identifier: Apache-2.0

//! Watches this node's pods and tracks the containers that use NVIDIA GPUs.

use super::cleanup::CleanupQueue;
use super::rules::{
    desired_gpu_containers_for_pod, finished_at_for_container, observed_finished_at_for_container,
    update_gpu_container,
};
use crate::config::Config;
use crate::log;
use crate::metrics::provider::GpuMetricsProvider;
use crate::model::{CalculatedValues, DeviceInfo, GpuContainer, GpuContainerKey};
use crate::process::Mapper;
use anyhow::{Context, anyhow, bail};
use futures::StreamExt;
use jiff::Timestamp;
use k8s_openapi::api::authorization::v1::{
    ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec,
};
use k8s_openapi::api::core::v1::Pod;
use kube::api::PostParams;
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Labels and values the collector needs to report one container on one device.
pub struct MetricLabelsAndValues {
    pub container_labels: Vec<String>,
    pub per_device_labels: Vec<String>,
    pub fingerprint: u64,
    pub calculated_values: Option<CalculatedValues>,
}

pub struct Store {
    state: Mutex<State>,
    mapper: Mapper,
    gmp: Arc<GpuMetricsProvider>,
}

struct State {
    /// Containers by pod UID, then container ID.
    containers: HashMap<String, HashMap<String, GpuContainer>>,
    cleanup: CleanupQueue,
}

impl Store {
    pub fn new(cfg: &Config, gmp: Arc<GpuMetricsProvider>) -> Self {
        Store {
            state: Mutex::new(State {
                containers: HashMap::new(),
                cleanup: CleanupQueue::new(cfg.scrape_interval),
            }),
            mapper: Mapper::new(&cfg.host_proc_mount_point),
            gmp,
        }
    }

    /// Connects (kubeconfig, then in-cluster), checks pod permissions, watches this node's pods until the initial
    /// sync, then starts the cleanup sweeper.
    pub async fn start(
        cfg: &Config,
        gmp: Arc<GpuMetricsProvider>,
        cancel: CancellationToken,
    ) -> anyhow::Result<Arc<Store>> {
        let client = Client::try_default()
            .await
            .context("create kubernetes client")?;
        verify_pod_access(&client).await?;

        let store = Arc::new(Store::new(cfg, gmp));
        let (synced_tx, synced_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(
            store
                .clone()
                .watch(client, cfg.node_name.clone(), synced_tx, cancel.clone()),
        );
        tokio::select! {
            synced = synced_rx => synced.context("sync pod informer: watch stopped")?,
            _ = cancel.cancelled() => bail!("sync pod informer: context canceled"),
        }
        tokio::spawn(
            store
                .clone()
                .sweep(CleanupQueue::sweep_interval(cfg.scrape_interval), cancel),
        );
        Ok(store)
    }

    #[cfg(test)]
    pub fn insert_for_test(&self, gc: GpuContainer) {
        let mut state = self.state.lock().unwrap();
        state
            .containers
            .entry(gc.key.pod_uid.clone())
            .or_default()
            .insert(gc.key.container_id.clone(), gc);
    }

    pub fn mapper(&self) -> &Mapper {
        &self.mapper
    }

    pub fn get_metric_labels_and_values(
        &self,
        key: &GpuContainerKey,
        dev_info: &DeviceInfo,
    ) -> Option<MetricLabelsAndValues> {
        let mut state = self.state.lock().unwrap();
        let gc = state
            .containers
            .get_mut(&key.pod_uid)?
            .get_mut(&key.container_id)?;
        let container_labels = gc.container_labels();
        let dmf = gc.device_labels.entry(dev_info.uuid.clone()).or_default();
        dmf.conditionally_set(&dev_info.uuid, &dev_info.model_name, &container_labels);
        let (per_device_labels, fingerprint) = (dmf.per_device_labels.clone(), dmf.fingerprint);
        let calculated_values = gc.get_calculated_values(dev_info);
        Some(MetricLabelsAndValues {
            container_labels,
            per_device_labels,
            fingerprint,
            calculated_values,
        })
    }

    fn reconcile_pod(&self, pod: &Pod) {
        let desired = desired_gpu_containers_for_pod(pod);
        let uid = pod.metadata.uid.clone().unwrap_or_default();
        let mut state = self.state.lock().unwrap();
        let State {
            containers,
            cleanup,
        } = &mut *state;

        for existing in containers
            .get_mut(&uid)
            .into_iter()
            .flat_map(|m| m.values_mut())
        {
            if !desired.contains_key(&existing.key.container_id) && existing.finished_at.is_none() {
                let finished_at = finished_at_for_container(pod, existing);
                self.finish_container_at(cleanup, existing, finished_at);
            }
        }

        for (id, mut container) in desired {
            if let Some(existing) = containers.get_mut(&uid).and_then(|m| m.get_mut(&id)) {
                update_gpu_container(existing, &container);
                if existing.finished_at.is_none() {
                    let finished_at = observed_finished_at_for_container(pod, existing);
                    self.finish_container_at(cleanup, existing, finished_at);
                }
                continue;
            }
            container.finished_at = observed_finished_at_for_container(pod, &container);
            cleanup.enqueue(&container);
            containers
                .entry(uid.clone())
                .or_default()
                .insert(id, container);
        }
    }

    fn finish_deleted_pod(&self, pod: &Pod) {
        let uid = pod.metadata.uid.as_deref().unwrap_or_default();
        let mut state = self.state.lock().unwrap();
        let State {
            containers,
            cleanup,
        } = &mut *state;
        for container in containers
            .get_mut(uid)
            .into_iter()
            .flat_map(|m| m.values_mut())
        {
            if container.finished_at.is_none() {
                let finished_at =
                    finished_at_for_container(pod, container).unwrap_or_else(Timestamp::now);
                self.finish_container_at(cleanup, container, Some(finished_at));
            }
        }
    }

    /// Marks the container finished, zeroes its memory gauges and schedules its removal.
    fn finish_container_at(
        &self,
        cleanup: &mut CleanupQueue,
        container: &mut GpuContainer,
        at: Option<Timestamp>,
    ) {
        let Some(at) = at else { return };
        container.finished_at = Some(at);
        self.gmp
            .zero_values(&container.all_per_device_label_fingerprints());
        cleanup.enqueue(container);
    }

    /// Removes the containers whose TTL expired, with their series and PID cache entries.
    fn purge(&self, now: Timestamp) {
        let mut expired = Vec::new();
        let mut container_labels = Vec::new();
        let mut per_device = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            let State {
                containers,
                cleanup,
            } = &mut *state;
            for key in cleanup.pop_expired(now) {
                let Some(by_id) = containers.get_mut(&key.pod_uid) else {
                    continue;
                };
                let Some(mut container) = by_id.remove(&key.container_id) else {
                    continue;
                };
                if by_id.is_empty() {
                    containers.remove(&key.pod_uid);
                }
                container_labels.push(container.container_labels());
                per_device.extend(container.all_per_device_label_fingerprints());
                expired.push(key);
            }
        }
        self.mapper.remove_container_keys(&expired);
        self.gmp.delete_values(&container_labels, &per_device);
    }

    async fn sweep(self: Arc<Self>, interval: std::time::Duration, cancel: CancellationToken) {
        let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => self.purge(Timestamp::now()),
                _ = cancel.cancelled() => return,
            }
        }
    }

    async fn watch(
        self: Arc<Self>,
        client: Client,
        node_name: String,
        synced: tokio::sync::oneshot::Sender<()>,
        cancel: CancellationToken,
    ) {
        let config = watcher::Config::default().fields(&format!("spec.nodeName={node_name}"));
        let mut events = watcher(Api::<Pod>::all(client), config)
            .default_backoff()
            .boxed();
        let mut informer = Informer::default();
        let mut synced = Some(synced);
        loop {
            let event = tokio::select! {
                event = events.next() => event,
                _ = cancel.cancelled() => return,
            };
            match event {
                Some(Ok(event)) => {
                    if informer.handle(&self, event)
                        && let Some(tx) = synced.take()
                    {
                        let _ = tx.send(());
                    }
                }
                Some(Err(e)) => log!("kube: pod watch failed: {e}"),
                None => return,
            }
        }
    }
}

/// Mirrors the client-go informer cache on top of the watcher: a relist that no longer contains a pod counts as
/// its deletion. Pods are keyed by namespace/name, like the informer.
#[derive(Default)]
struct Informer {
    known: HashMap<(String, String), Pod>,
    listing: HashSet<(String, String)>,
}

impl Informer {
    /// Applies a watcher event and returns true when an initial list or relist completes.
    fn handle(&mut self, store: &Store, event: watcher::Event<Pod>) -> bool {
        let key = |pod: &Pod| {
            (
                pod.metadata.namespace.clone().unwrap_or_default(),
                pod.metadata.name.clone().unwrap_or_default(),
            )
        };
        match event {
            watcher::Event::Init => self.listing.clear(),
            watcher::Event::InitApply(pod) => {
                self.listing.insert(key(&pod));
                store.reconcile_pod(&pod);
                self.known.insert(key(&pod), pod);
            }
            watcher::Event::InitDone => {
                let gone: Vec<_> = self
                    .known
                    .keys()
                    .filter(|k| !self.listing.contains(*k))
                    .cloned()
                    .collect();
                for k in gone {
                    store.finish_deleted_pod(&self.known.remove(&k).unwrap());
                }
                return true;
            }
            watcher::Event::Apply(pod) => {
                store.reconcile_pod(&pod);
                self.known.insert(key(&pod), pod);
            }
            watcher::Event::Delete(pod) => {
                self.known.remove(&key(&pod));
                store.finish_deleted_pod(&pod);
            }
        }
        false
    }
}

async fn verify_pod_access(client: &Client) -> anyhow::Result<()> {
    let api = Api::<SelfSubjectAccessReview>::all(client.clone());
    for verb in ["get", "list", "watch"] {
        let review = SelfSubjectAccessReview {
            spec: SelfSubjectAccessReviewSpec {
                resource_attributes: Some(ResourceAttributes {
                    group: Some(String::new()),
                    resource: Some("pods".into()),
                    verb: Some(verb.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let review = api
            .create(&PostParams::default(), &review)
            .await
            .with_context(|| format!("check {verb} permission"))?;
        let status = review.status.unwrap_or_default();
        if !status.allowed {
            let reason = status.reason.unwrap_or_default();
            let reason = if reason.trim().is_empty() {
                "access denied"
            } else {
                reason.trim()
            };
            return Err(anyhow!("missing {verb} permission for pods: {reason}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kube::cleanup::tests::test_container;
    use crate::kube::rules::tests::*;
    use crate::kube::rules::{
        KAI_FRACTION_ANNOTATION, KAI_FRACTION_CONTAINER_ANNOTATION, KAI_MEMORY_ANNOTATION,
    };
    use crate::metrics::provider::{set_value, tests::gauge_value};
    use crate::model::{DeviceModelFingerprint, GpuAllocationType};
    use prometheus::Registry;
    use serde_json::json;
    use std::time::Duration;

    struct TestStore {
        store: Store,
        informer: Informer,
    }

    impl TestStore {
        fn start(pods: Vec<Pod>) -> Self {
            let cfg = Config {
                node_name: TEST_NODE_NAME.into(),
                host_proc_mount_point: "/nonexistent".into(),
                driver_poll_interval: Duration::from_secs(1),
                scrape_interval: Duration::from_secs(1),
                exporter_endpoint: "/metrics".into(),
                exporter_port: 9494,
            };
            let mut ts = TestStore {
                store: Store::new(&cfg, Arc::new(GpuMetricsProvider::new(&Registry::new()))),
                informer: Informer::default(),
            };
            ts.event(watcher::Event::Init);
            for pod in pods {
                ts.event(watcher::Event::InitApply(pod));
            }
            assert!(ts.informer.handle(&ts.store, watcher::Event::InitDone));
            ts
        }

        fn event(&mut self, event: watcher::Event<Pod>) {
            self.informer.handle(&self.store, event);
        }

        fn total(&self) -> usize {
            self.store
                .state
                .lock()
                .unwrap()
                .containers
                .values()
                .map(HashMap::len)
                .sum()
        }

        fn cleanup_len(&self) -> usize {
            self.store.state.lock().unwrap().cleanup.len()
        }

        fn container(&self, uid: &str, id: &str) -> Option<GpuContainer> {
            self.store
                .state
                .lock()
                .unwrap()
                .containers
                .get(uid)?
                .get(id)
                .cloned()
        }
    }

    fn standard_gpu_pod(name: &str, namespace: &str, container: &str, id: &str) -> Pod {
        pod(json!({
            "metadata": {"name": name, "namespace": namespace, "uid": format!("{name}-uid")},
            "spec": {"nodeName": TEST_NODE_NAME, "containers": [{"name": container,
                     "resources": {"requests": {"nvidia.com/gpu": "1"}, "limits": {"nvidia.com/gpu": "1"}}}]},
            "status": {"containerStatuses": [running_status(container, id)]},
        }))
    }

    fn kai_memory_pod(name: &str, container: &str, id: &str, memory: u64) -> Pod {
        pod(json!({
            "metadata": {"name": name, "namespace": TEST_NAMESPACE, "uid": format!("{name}-uid"),
                         "annotations": {KAI_MEMORY_ANNOTATION: memory.to_string(),
                                         KAI_FRACTION_CONTAINER_ANNOTATION: container}},
            "spec": {"nodeName": TEST_NODE_NAME, "containers": [{"name": "sidecar"}, {"name": container}]},
            "status": {"containerStatuses": [running_status("sidecar", &"7".repeat(64)), running_status(container, id)]},
        }))
    }

    fn kai_init_container_pod(name: &str, container: &str, id: &str, fraction: f64) -> Pod {
        pod(json!({
            "metadata": {"name": name, "namespace": TEST_NAMESPACE, "uid": format!("{name}-uid"),
                         "annotations": {KAI_FRACTION_ANNOTATION: fraction.to_string(),
                                         KAI_FRACTION_CONTAINER_ANNOTATION: container}},
            "spec": {"nodeName": TEST_NODE_NAME, "containers": [{"name": MAIN_CONTAINER_NAME}],
                     "initContainers": [{"name": container, "restartPolicy": "Always"}]},
            "status": {"containerStatuses": [running_status(MAIN_CONTAINER_NAME, &"8".repeat(64))],
                       "initContainerStatuses": [running_status(container, id)]},
        }))
    }

    fn without_kai_fraction(pod: &Pod, statuses: serde_json::Value) -> Pod {
        let mut updated = pod.clone();
        updated
            .metadata
            .annotations
            .as_mut()
            .unwrap()
            .remove(KAI_FRACTION_ANNOTATION);
        updated.status.as_mut().unwrap().container_statuses =
            serde_json::from_value(statuses).unwrap();
        updated
    }

    #[test]
    fn store_initial_sync_finds_relevant_pods() {
        let ts = TestStore::start(vec![
            standard_gpu_pod(
                "reservation-pod",
                "kai-resource-reservation",
                "reservation-container",
                &"a".repeat(64),
            ),
            kai_fraction_pod("kai-fraction-pod", &"b".repeat(64), 0.25),
            kai_memory_pod("kai-memory-pod", "worker", &"c".repeat(64), 4096),
            kai_init_container_pod(
                "kai-init-pod",
                KAI_INIT_CONTAINER_NAME,
                &"d".repeat(64),
                0.5,
            ),
            standard_gpu_pod("gpu-pod", TEST_NAMESPACE, "gpu-worker", &"e".repeat(64)),
            irrelevant_pod("ignored-pod"),
        ]);
        assert_eq!(ts.total(), 4);

        let gc = ts
            .container("kai-fraction-pod-uid", &"b".repeat(64))
            .unwrap();
        assert_eq!(
            (gc.gpu_allocation_type, gc.gpu_fraction),
            (GpuAllocationType::KaiScheduler, Some(0.25))
        );
        let gc = ts.container("kai-memory-pod-uid", &"c".repeat(64)).unwrap();
        assert_eq!(
            (gc.container_name.as_str(), gc.gpu_memory),
            ("worker", Some(4096))
        );
        let gc = ts.container("kai-init-pod-uid", &"d".repeat(64)).unwrap();
        assert_eq!(
            (gc.container_name.as_str(), gc.gpu_fraction),
            (KAI_INIT_CONTAINER_NAME, Some(0.5))
        );
        let gc = ts.container("gpu-pod-uid", &"e".repeat(64)).unwrap();
        assert_eq!(gc.gpu_allocation_type, GpuAllocationType::K8sResource);
        assert_eq!((gc.gpu_request, gc.gpu_limit), (Some(1), Some(1)));
        assert!(
            ts.container("reservation-pod-uid", &"a".repeat(64))
                .is_none()
        );
    }

    #[test]
    fn store_handles_additions_after_start() {
        let mut ts = TestStore::start(vec![]);
        assert_eq!(ts.total(), 0);
        let reservation = standard_gpu_pod(
            "reservation-added",
            "kai-resource-reservation",
            "reservation-container",
            &"1".repeat(64),
        );
        ts.event(watcher::Event::Apply(reservation));
        ts.event(watcher::Event::Apply(irrelevant_pod("ignored-added")));
        ts.event(watcher::Event::Apply(standard_gpu_pod(
            "gpu-added",
            TEST_NAMESPACE,
            "gpu-worker",
            &"2".repeat(64),
        )));
        assert_eq!(ts.total(), 1);
        assert!(ts.container("gpu-added-uid", &"2".repeat(64)).is_some());
    }

    #[test]
    fn store_marks_deleted_relevant_pod_finished() {
        let pod = kai_fraction_pod("kai-delete-pod", &"3".repeat(64), 0.75);
        let mut ts = TestStore::start(vec![pod.clone()]);
        ts.event(watcher::Event::Delete(pod));
        assert!(
            ts.container("kai-delete-pod-uid", &"3".repeat(64))
                .unwrap()
                .finished_at
                .is_some()
        );
    }

    #[test]
    fn store_marks_pod_missing_from_relist_finished() {
        let pod = kai_fraction_pod("kai-relist-pod", &"3".repeat(64), 0.75);
        let mut ts = TestStore::start(vec![pod]);
        ts.event(watcher::Event::Init);
        assert!(ts.informer.handle(&ts.store, watcher::Event::InitDone));
        assert!(
            ts.container("kai-relist-pod-uid", &"3".repeat(64))
                .unwrap()
                .finished_at
                .is_some()
        );
    }

    fn store_tracks_restarted_relevant_pod(exit_code: i32) {
        let pod = kai_fraction_pod("kai-restart-pod", &"5".repeat(64), 0.625);
        let mut ts = TestStore::start(vec![pod.clone()]);
        let (old, new) = ("5".repeat(64), "6".repeat(64));
        let mut restarted = pod.clone();
        let finished = (Timestamp::now() - Duration::from_secs(60)).to_string();
        restarted.status.as_mut().unwrap().container_statuses = serde_json::from_value(json!([{
            "name": MAIN_CONTAINER_NAME, "containerID": format!("containerd://{new}"), "image": "", "imageID": "",
            "ready": true, "restartCount": 1, "state": {"running": {}},
            "lastState": {"terminated": {"containerID": format!("containerd://{old}"), "exitCode": exit_code,
                                         "finishedAt": finished}},
        }]))
        .unwrap();
        ts.event(watcher::Event::Apply(restarted));
        assert_eq!(ts.total(), 2);
        assert!(
            ts.container("kai-restart-pod-uid", &old)
                .unwrap()
                .finished_at
                .is_some()
        );
        assert!(
            ts.container("kai-restart-pod-uid", &new)
                .unwrap()
                .finished_at
                .is_none()
        );
    }

    #[test]
    fn store_tracks_restarted_relevant_pod_exit_code_zero() {
        store_tracks_restarted_relevant_pod(0);
    }

    #[test]
    fn store_tracks_restarted_relevant_pod_exit_code137() {
        store_tracks_restarted_relevant_pod(137);
    }

    #[test]
    fn store_tracks_kai_annotation_added_to_irrelevant_pod() {
        let pod = irrelevant_pod("became-kai");
        let mut ts = TestStore::start(vec![pod.clone()]);
        assert_eq!(ts.total(), 0);
        let mut updated = pod;
        updated.metadata.annotations =
            Some([(KAI_FRACTION_ANNOTATION.to_string(), "0.125".to_string())].into());
        ts.event(watcher::Event::Apply(updated));
        let gc = ts.container("became-kai-uid", &"9".repeat(64)).unwrap();
        assert_eq!(
            (gc.gpu_allocation_type, gc.gpu_fraction),
            (GpuAllocationType::KaiScheduler, Some(0.125))
        );
    }

    #[test]
    fn store_marks_relevant_pod_finished_when_kai_annotation_removed() {
        let id = "4".repeat(64);
        let pod = kai_fraction_pod("kai-remove-pod", &id, 0.875);
        let mut ts = TestStore::start(vec![pod.clone()]);
        ts.event(watcher::Event::Apply(without_kai_fraction(
            &pod,
            json!([terminated_status(MAIN_CONTAINER_NAME, &id)]),
        )));
        assert!(
            ts.container("kai-remove-pod-uid", &id)
                .unwrap()
                .finished_at
                .is_some()
        );
        assert_eq!(ts.cleanup_len(), 1);
    }

    #[test]
    fn store_does_not_finish_container_without_terminal_evidence() {
        let id = "7".repeat(64);
        let pod = kai_fraction_pod("kai-status-lag-pod", &id, 0.5);
        let mut ts = TestStore::start(vec![pod.clone()]);
        let mut updated = without_kai_fraction(&pod, json!(null));
        updated.status.as_mut().unwrap().phase = Some("Running".into());
        ts.event(watcher::Event::Apply(updated));
        assert!(
            ts.container("kai-status-lag-pod-uid", &id)
                .unwrap()
                .finished_at
                .is_none()
        );
        assert_eq!(ts.cleanup_len(), 0);
    }

    #[test]
    fn store_cleanup_scheduler_tracks_finished_containers() {
        let id = "12".repeat(32);
        let pod = kai_fraction_pod("kai-cleanup-pod", &id, 0.375);
        let mut ts = TestStore::start(vec![pod.clone()]);
        assert_eq!(ts.cleanup_len(), 0);
        let updated =
            without_kai_fraction(&pod, json!([terminated_status(MAIN_CONTAINER_NAME, &id)]));
        ts.event(watcher::Event::Apply(updated.clone()));
        assert_eq!(ts.cleanup_len(), 1);
        ts.event(watcher::Event::Apply(updated));
        assert_eq!(
            ts.cleanup_len(),
            1,
            "the same finished container stays scheduled once"
        );
    }

    #[test]
    fn store_zeroes_current_gauges_when_container_finishes() {
        let id = "13".repeat(32);
        let pod = kai_fraction_pod("kai-zero-pod", &id, 0.5);
        let mut ts = TestStore::start(vec![pod.clone()]);
        let labels = {
            let mut state = ts.store.state.lock().unwrap();
            let gc = state
                .containers
                .get_mut("kai-zero-pod-uid")
                .unwrap()
                .get_mut(&id)
                .unwrap();
            gc.device_labels
                .insert("gpu-a".into(), DeviceModelFingerprint::new("model-a"));
            gc.per_device_labels("gpu-a")
        };
        let gmp = ts.store.gmp.clone();
        set_value(&gmp.memory_bytes, &labels, 512.0);
        set_value(&gmp.memory_footprint, &labels, 25.0);
        set_value(&gmp.protected_memory_bytes, &labels, 128.0);

        ts.event(watcher::Event::Apply(without_kai_fraction(
            &pod,
            json!([terminated_status(MAIN_CONTAINER_NAME, &id)]),
        )));

        assert!(
            ts.container("kai-zero-pod-uid", &id)
                .unwrap()
                .finished_at
                .is_some()
        );
        for gauge in [
            &gmp.memory_bytes,
            &gmp.memory_footprint,
            &gmp.protected_memory_bytes,
        ] {
            assert_eq!(gauge_value(gauge, &labels), 0.0);
        }
    }

    #[test]
    fn cleanup_scheduler_purge_removes_expired_containers() {
        let ts = TestStore::start(vec![]);
        let now = Timestamp::now();
        let expired = test_container(
            "expired-pod",
            &"c".repeat(64),
            Some(now - Duration::from_secs(2)),
        );
        let active = test_container("active-pod", &"d".repeat(64), Some(now));
        {
            let mut state = ts.store.state.lock().unwrap();
            for gc in [&expired, &active] {
                state.cleanup.enqueue(gc);
                state
                    .containers
                    .entry(gc.key.pod_uid.clone())
                    .or_default()
                    .insert(gc.key.container_id.clone(), gc.clone());
            }
        }

        // The test TTL is 1.5s, so only the container finished 2s ago has expired.
        ts.store.purge(Timestamp::now());

        assert!(ts.container("expired-pod", &"c".repeat(64)).is_none());
        assert!(ts.container("active-pod", &"d".repeat(64)).is_some());
        assert_eq!(ts.cleanup_len(), 1);
    }
}
