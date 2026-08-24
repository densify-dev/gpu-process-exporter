// SPDX-License-Identifier: Apache-2.0

package kube

import (
	"testing"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	gpuprom "github.com/densify-dev/gpu-process-exporter/pkg/prometheus"
	prom "github.com/prometheus/client_golang/prometheus"
	"k8s.io/apimachinery/pkg/types"
)

const testMetricHelp = "test metric"

func TestCleanupSchedulerEnqueueSkipsZeroAndDuplicateContainers(t *testing.T) {
	store := newCleanupSchedulerTestStore()
	scheduler := NewCleanupScheduler(store)

	running := newTestGpuContainer(
		"running-pod",
		"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		time.Time{},
	)
	scheduler.enqueue(nil)
	scheduler.enqueue(running)

	finishedAt := time.Now().Add(-time.Minute)
	finished := newTestGpuContainer(
		"finished-pod",
		"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
		finishedAt,
	)
	scheduler.enqueue(finished)
	scheduler.enqueue(finished)

	if got := len(scheduler.gpuContainers); got != 1 {
		t.Fatalf("unexpected heap size: got %d want 1", got)
	}
	if got := len(scheduler.queued); got != 1 {
		t.Fatalf("unexpected queued size: got %d want 1", got)
	}
	if _, ok := scheduler.queued[finished.String()]; !ok {
		t.Fatalf("finished container was not enqueued")
	}
}

func TestCleanupSchedulerPurgeRemovesExpiredContainers(t *testing.T) {
	store := newCleanupSchedulerTestStore()
	scheduler := NewCleanupScheduler(store)
	scheduler.ttl = time.Second
	store.cleanupScheduler = scheduler

	expired := newTestGpuContainer(
		"expired-pod",
		"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
		time.Now().Add(-2*time.Second),
	)
	active := newTestGpuContainer(
		"active-pod",
		"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
		time.Now(),
	)

	store.gpuContainers[expired.PodUid] = map[string]*model.GpuContainer{expired.ContainerId: expired}
	store.gpuContainers[active.PodUid] = map[string]*model.GpuContainer{active.ContainerId: active}

	scheduler.enqueue(expired)
	scheduler.enqueue(active)

	scheduler.purge()

	if _, ok := store.gpuContainers[expired.PodUid][expired.ContainerId]; ok {
		t.Fatalf("expired container was not removed from store")
	}
	if _, ok := scheduler.queued[expired.String()]; ok {
		t.Fatalf("expired container was not removed from scheduler")
	}
	if _, ok := store.gpuContainers[active.PodUid][active.ContainerId]; !ok {
		t.Fatalf("active container was removed from store")
	}
	if _, ok := scheduler.queued[active.String()]; !ok {
		t.Fatalf("active container was removed from scheduler")
	}
}

func newCleanupSchedulerTestStore() *Store {
	store := &Store{
		cfg:           &config.Config{ScrapeInterval: 50 * time.Millisecond},
		gpuContainers: make(map[types.UID]map[string]*model.GpuContainer),
		gmp: &gpuprom.GpuMetricsProvider{
			Requests: prom.NewGaugeVec(prom.GaugeOpts{
				Name: "test_kubex_gpu_container_requests",
				Help: testMetricHelp,
			}, []string{
				gpuprom.LabelNode,
				gpuprom.LabelNamespace,
				gpuprom.LabelPod,
				gpuprom.LabelContainer,
				gpuprom.LabelContainerId,
				gpuprom.LabelGpuAllocationType,
			}),
			Limits: prom.NewGaugeVec(prom.GaugeOpts{
				Name: "test_kubex_gpu_container_limits",
				Help: testMetricHelp,
			}, []string{
				gpuprom.LabelNode,
				gpuprom.LabelNamespace,
				gpuprom.LabelPod,
				gpuprom.LabelContainer,
				gpuprom.LabelContainerId,
				gpuprom.LabelGpuAllocationType,
			}),
		},
	}
	return store
}

func newTestGpuContainer(podUID, containerID string, finishedAt time.Time) *model.GpuContainer {
	return &model.GpuContainer{
		GpuContainerKey: &model.GpuContainerKey{
			PodUid:      types.UID(podUID),
			ContainerId: containerID,
		},
		FinishedAt: finishedAt,
	}
}
