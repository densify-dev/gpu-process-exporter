// SPDX-License-Identifier: Apache-2.0

package kube

import (
	"context"
	"fmt"
	"testing"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	gpuprom "github.com/densify-dev/gpu-process-exporter/pkg/prometheus"
	clientprom "github.com/prometheus/client_golang/prometheus"
	dto "github.com/prometheus/client_model/go"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/api/resource"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/informers"
	kubefake "k8s.io/client-go/kubernetes/fake"
	"k8s.io/client-go/tools/cache"
)

const testNodeName = "node-a"

const (
	testNamespace        = "test"
	mainContainerName    = "main"
	kaiInitContainerName = "kai-init"
	testMetricHelpStore  = "test metric"
)

func TestStoreInitialSyncFindsRelevantPods(t *testing.T) {
	store, _, stop := startTestStore(t,
		newKaiReservationPod(
			"reservation-pod",
			"reservation-container",
			"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		),
		newKaiFractionPod(
			"kai-fraction-pod",
			"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
			0.25,
		),
		newKaiMemoryPod(
			"kai-memory-pod",
			"worker",
			"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
			4096,
		),
		newKaiInitContainerPod(
			"kai-init-pod",
			kaiInitContainerName,
			"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
			0.5,
		),
		newStandardGPUPod(
			"gpu-pod",
			"gpu-worker",
			"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
		),
		newIrrelevantPod("ignored-pod"),
	)
	defer stop()

	waitForGPUContainerTotal(t, store, 4)

	assertContainer(t, store, &model.GpuContainerKey{
		PodUid:      types.UID("kai-fraction-pod-uid"),
		ContainerId: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
	}, func(gc *model.GpuContainer) {
		if gc.GpuAllocationType != model.KaiScheduler {
			t.Fatalf("unexpected allocation type: got %v", gc.GpuAllocationType)
		}
		if gc.GpuFraction == nil || *gc.GpuFraction != 0.25 {
			t.Fatalf("unexpected gpu fraction: %+v", gc.GpuFraction)
		}
	})

	assertContainer(t, store, &model.GpuContainerKey{
		PodUid:      types.UID("kai-memory-pod-uid"),
		ContainerId: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
	}, func(gc *model.GpuContainer) {
		if gc.GpuAllocationType != model.KaiScheduler {
			t.Fatalf("unexpected allocation type: got %v", gc.GpuAllocationType)
		}
		if gc.ContainerName != "worker" {
			t.Fatalf("unexpected container name: %s", gc.ContainerName)
		}
		if gc.GpuMemory == nil || *gc.GpuMemory != 4096 {
			t.Fatalf("unexpected gpu memory: %+v", gc.GpuMemory)
		}
	})

	assertContainer(t, store, &model.GpuContainerKey{
		PodUid:      types.UID("kai-init-pod-uid"),
		ContainerId: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
	}, func(gc *model.GpuContainer) {
		if gc.GpuAllocationType != model.KaiScheduler {
			t.Fatalf("unexpected allocation type: got %v", gc.GpuAllocationType)
		}
		if gc.ContainerName != kaiInitContainerName {
			t.Fatalf("unexpected container name: %s", gc.ContainerName)
		}
		if gc.GpuFraction == nil || *gc.GpuFraction != 0.5 {
			t.Fatalf("unexpected gpu fraction: %+v", gc.GpuFraction)
		}
	})

	assertContainer(t, store, &model.GpuContainerKey{
		PodUid:      types.UID("gpu-pod-uid"),
		ContainerId: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
	}, func(gc *model.GpuContainer) {
		if gc.GpuAllocationType != model.K8sResource {
			t.Fatalf("unexpected allocation type: got %v", gc.GpuAllocationType)
		}
		if gc.GpuRequest == nil || *gc.GpuRequest != 1 {
			t.Fatalf("unexpected gpu request: %+v", gc.GpuRequest)
		}
		if gc.GpuLimit == nil || *gc.GpuLimit != 1 {
			t.Fatalf("unexpected gpu limit: %+v", gc.GpuLimit)
		}
	})

	if _, ok := store.get(&model.GpuContainerKey{
		PodUid:      types.UID("reservation-pod-uid"),
		ContainerId: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
	}); ok {
		t.Fatalf("kai reservation pod should not be tracked")
	}
}

func TestStoreHandlesAdditionsAfterStart(t *testing.T) {
	store, client, stop := startTestStore(t)
	defer stop()

	waitForGPUContainerTotal(t, store, 0)

	if _, err := client.CoreV1().Pods(KaiResourceReservationNamespace).Create(
		t.Context(),
		newKaiReservationPod(
			"reservation-added",
			"reservation-container",
			"1111111111111111111111111111111111111111111111111111111111111111",
		),
		metav1.CreateOptions{},
	); err != nil {
		t.Fatalf("create reservation pod: %v", err)
	}
	if _, err := client.CoreV1().Pods(testNamespace).Create(
		t.Context(),
		newIrrelevantPod("ignored-added"),
		metav1.CreateOptions{},
	); err != nil {
		t.Fatalf("create irrelevant pod: %v", err)
	}
	if _, err := client.CoreV1().Pods(testNamespace).Create(
		t.Context(),
		newStandardGPUPod(
			"gpu-added",
			"gpu-worker",
			"2222222222222222222222222222222222222222222222222222222222222222",
		),
		metav1.CreateOptions{},
	); err != nil {
		t.Fatalf("create gpu pod: %v", err)
	}

	waitForGPUContainerTotal(t, store, 1)
	assertContainer(t, store, &model.GpuContainerKey{
		PodUid:      types.UID("gpu-added-uid"),
		ContainerId: "2222222222222222222222222222222222222222222222222222222222222222",
	}, nil)
}

func TestStoreMarksDeletedRelevantPodFinished(t *testing.T) {
	pod := newKaiFractionPod("kai-delete-pod", "3333333333333333333333333333333333333333333333333333333333333333", 0.75)
	store, client, stop := startTestStore(t, pod)
	defer stop()

	key := &model.GpuContainerKey{
		PodUid:      pod.UID,
		ContainerId: "3333333333333333333333333333333333333333333333333333333333333333",
	}
	waitForGPUContainerTotal(t, store, 1)

	if err := client.CoreV1().Pods(pod.Namespace).Delete(t.Context(), pod.Name, metav1.DeleteOptions{}); err != nil {
		t.Fatalf("delete pod: %v", err)
	}

	waitForContainerState(t, store, key, func(gc *model.GpuContainer) bool {
		return !gc.FinishedAt.IsZero()
	})
}

func TestStoreTracksRestartedRelevantPodExitCodeZero(t *testing.T) {
	testStoreTracksRestartedRelevantPod(t, 0)
}

func TestStoreTracksRestartedRelevantPodExitCode137(t *testing.T) {
	testStoreTracksRestartedRelevantPod(t, 137)
}

func TestStoreTracksKaiAnnotationAddedToIrrelevantPod(t *testing.T) {
	pod := newIrrelevantPod("became-kai")
	store, client, stop := startTestStore(t, pod)
	defer stop()

	waitForGPUContainerTotal(t, store, 0)

	updated := pod.DeepCopy()
	updated.Annotations = map[string]string{
		KaiFractionAnnotation: "0.125",
	}
	if _, err := client.CoreV1().Pods(updated.Namespace).Update(t.Context(), updated, metav1.UpdateOptions{}); err != nil {
		t.Fatalf("update pod: %v", err)
	}

	waitForGPUContainerTotal(t, store, 1)
	assertContainer(t, store, &model.GpuContainerKey{
		PodUid:      updated.UID,
		ContainerId: "9999999999999999999999999999999999999999999999999999999999999999",
	}, func(gc *model.GpuContainer) {
		if gc.GpuAllocationType != model.KaiScheduler {
			t.Fatalf("unexpected allocation type: got %v", gc.GpuAllocationType)
		}
		if gc.GpuFraction == nil || *gc.GpuFraction != 0.125 {
			t.Fatalf("unexpected gpu fraction: %+v", gc.GpuFraction)
		}
	})
}

func TestStoreMarksRelevantPodFinishedWhenKaiAnnotationRemoved(t *testing.T) {
	pod := newKaiFractionPod("kai-remove-pod", "4444444444444444444444444444444444444444444444444444444444444444", 0.875)
	store, client, stop := startTestStore(t, pod)
	defer stop()

	key := &model.GpuContainerKey{
		PodUid:      pod.UID,
		ContainerId: "4444444444444444444444444444444444444444444444444444444444444444",
	}
	waitForGPUContainerTotal(t, store, 1)

	updated := pod.DeepCopy()
	delete(updated.Annotations, KaiFractionAnnotation)
	updated.Status.ContainerStatuses = []corev1.ContainerStatus{
		terminatedStatus(mainContainerName, key.ContainerId),
	}
	if _, err := client.CoreV1().Pods(updated.Namespace).Update(t.Context(), updated, metav1.UpdateOptions{}); err != nil {
		t.Fatalf("update pod: %v", err)
	}

	waitForContainerState(t, store, key, func(gc *model.GpuContainer) bool {
		return !gc.FinishedAt.IsZero()
	})
	waitForCleanupSchedulerSize(t, store, 1)
}

func TestStoreDoesNotFinishContainerWithoutTerminalEvidence(t *testing.T) {
	pod := newKaiFractionPod("kai-status-lag-pod", "7777777777777777777777777777777777777777777777777777777777777777", 0.5)
	store, client, stop := startTestStore(t, pod)
	defer stop()

	key := &model.GpuContainerKey{
		PodUid:      pod.UID,
		ContainerId: "7777777777777777777777777777777777777777777777777777777777777777",
	}
	waitForGPUContainerTotal(t, store, 1)

	updated := pod.DeepCopy()
	delete(updated.Annotations, KaiFractionAnnotation)
	updated.Status.ContainerStatuses = nil
	updated.Status.Phase = corev1.PodRunning
	if _, err := client.CoreV1().Pods(updated.Namespace).Update(t.Context(), updated, metav1.UpdateOptions{}); err != nil {
		t.Fatalf("update pod: %v", err)
	}

	assertContainer(t, store, key, func(gc *model.GpuContainer) {
		if !gc.FinishedAt.IsZero() {
			t.Fatalf("FinishedAt = %v, want zero", gc.FinishedAt)
		}
	})
	waitForCleanupSchedulerSize(t, store, 0)
}

func TestStoreCleanupSchedulerTracksFinishedContainers(t *testing.T) {
	pod := newKaiFractionPod("kai-cleanup-pod", "1212121212121212121212121212121212121212121212121212121212121212", 0.375)
	store, client, stop := startTestStore(t, pod)
	defer stop()

	waitForGPUContainerTotal(t, store, 1)
	waitForCleanupSchedulerSize(t, store, 0)

	updated := pod.DeepCopy()
	delete(updated.Annotations, KaiFractionAnnotation)
	updated.Status.ContainerStatuses = []corev1.ContainerStatus{
		terminatedStatus(mainContainerName, "1212121212121212121212121212121212121212121212121212121212121212"),
	}
	if _, err := client.CoreV1().Pods(updated.Namespace).Update(t.Context(), updated, metav1.UpdateOptions{}); err != nil {
		t.Fatalf("update pod: %v", err)
	}

	waitForCleanupSchedulerSize(t, store, 1)

	if _, err := client.CoreV1().Pods(updated.Namespace).Update(t.Context(), updated, metav1.UpdateOptions{}); err != nil {
		t.Fatalf("repeat update pod: %v", err)
	}

	// The same finished container should stay scheduled exactly once.
	waitForCleanupSchedulerSize(t, store, 1)
}

func TestStoreZeroesCurrentGaugesWhenContainerFinishes(t *testing.T) {
	pod := newKaiFractionPod("kai-zero-pod", "1313131313131313131313131313131313131313131313131313131313131313", 0.5)
	store, client, stop := startTestStore(t, pod)
	defer stop()
	store.gmp = newStoreTestGpuMetricsProvider()

	key := &model.GpuContainerKey{
		PodUid:      pod.UID,
		ContainerId: "1313131313131313131313131313131313131313131313131313131313131313",
	}
	waitForGPUContainerTotal(t, store, 1)

	var labels []string
	store.gpuContainersMu.Lock()
	container := store.gpuContainers[key.PodUid][key.ContainerId]
	container.DeviceLabels["gpu-a"] = &model.DeviceModelFingerprint{ModelName: "model-a"}
	labels = container.GetPerDeviceLabels("gpu-a")
	store.gpuContainersMu.Unlock()

	gpuprom.SetValue(store.gmp.MemoryBytes, labels, 512)
	gpuprom.SetValue(store.gmp.MemoryFootprint, labels, 25)
	gpuprom.SetValue(store.gmp.ProtectedMemoryBytes, labels, 128)

	updated := pod.DeepCopy()
	delete(updated.Annotations, KaiFractionAnnotation)
	updated.Status.ContainerStatuses = []corev1.ContainerStatus{
		terminatedStatus(mainContainerName, key.ContainerId),
	}
	if _, err := client.CoreV1().Pods(updated.Namespace).Update(t.Context(), updated, metav1.UpdateOptions{}); err != nil {
		t.Fatalf("update pod: %v", err)
	}

	waitForContainerState(t, store, key, func(gc *model.GpuContainer) bool {
		return !gc.FinishedAt.IsZero()
	})

	for name, metric := range map[string]clientprom.Metric{
		"MemoryBytes":          store.gmp.MemoryBytes.WithLabelValues(labels...),
		"MemoryFootprint":      store.gmp.MemoryFootprint.WithLabelValues(labels...),
		"ProtectedMemoryBytes": store.gmp.ProtectedMemoryBytes.WithLabelValues(labels...),
	} {
		if got := storeTestGaugeValue(t, metric); got != 0 {
			t.Fatalf("%s = %v, want 0", name, got)
		}
	}
}

func TestDesiredGpuContainersForPodTracksKaiNumDevices(t *testing.T) {
	pod := newIrrelevantPod("kai-num-devices")
	pod.Annotations = map[string]string{
		KaiNumDevicesAnnotation: "2",
	}

	desired := desiredGpuContainersForPod(pod)
	if len(desired) != 1 {
		t.Fatalf("len(desired) = %d, want 1", len(desired))
	}

	var gc *model.GpuContainer
	for _, container := range desired {
		gc = container
	}
	if gc == nil {
		t.Fatal("expected gpu container")
	}
	if gc.GpuAllocationType != model.KaiScheduler {
		t.Fatalf("GpuAllocationType = %v, want %v", gc.GpuAllocationType, model.KaiScheduler)
	}
	if gc.GPUNumDevices == nil || *gc.GPUNumDevices != 2 {
		t.Fatalf("GPUNumDevices = %+v, want 2", gc.GPUNumDevices)
	}
}

func TestDesiredGpuContainersForPodSkipsKaiInitContainerWithoutAlwaysRestartPolicy(t *testing.T) {
	restartPolicy := corev1.ContainerRestartPolicyOnFailure
	pod := &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "kai-init-skip",
			Namespace: testNamespace,
			UID:       types.UID("kai-init-skip-uid"),
			Annotations: map[string]string{
				KaiFractionAnnotation:          "0.5",
				KaiFractionContainerAnnotation: kaiInitContainerName,
			},
		},
		Spec: corev1.PodSpec{
			NodeName: testNodeName,
			Containers: []corev1.Container{
				{Name: mainContainerName},
			},
			InitContainers: []corev1.Container{
				{
					Name:          kaiInitContainerName,
					RestartPolicy: &restartPolicy,
				},
			},
		},
		Status: corev1.PodStatus{
			ContainerStatuses: []corev1.ContainerStatus{
				runningStatus(mainContainerName, "abababababababababababababababababababababababababababababababab"),
			},
			InitContainerStatuses: []corev1.ContainerStatus{
				runningStatus(kaiInitContainerName, "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"),
			},
		},
	}

	if desired := desiredGpuContainersForPod(pod); len(desired) != 0 {
		t.Fatalf("len(desired) = %d, want 0", len(desired))
	}
}

func TestUpdateGpuContainerCopiesLimitAndNumDevices(t *testing.T) {
	oldRequest := int64(1)
	oldLimit := int64(1)
	oldNumDevices := uint64(1)
	dst := &model.GpuContainer{
		Namespace:     "old-ns",
		PodName:       "old-pod",
		ContainerName: "old-container",
		GpuRequest:    &oldRequest,
		GpuLimit:      &oldLimit,
		GPUNumDevices: &oldNumDevices,
	}

	newRequest := int64(2)
	newLimit := int64(3)
	newNumDevices := uint64(4)
	src := &model.GpuContainer{
		Namespace:     "new-ns",
		PodName:       "new-pod",
		ContainerName: "new-container",
		GpuRequest:    &newRequest,
		GpuLimit:      &newLimit,
		GPUNumDevices: &newNumDevices,
	}

	updateGpuContainer(dst, src)

	if dst.Namespace != "new-ns" || dst.PodName != "new-pod" || dst.ContainerName != "new-container" {
		t.Fatalf("basic fields not updated: %+v", dst)
	}
	if dst.GpuRequest == nil || *dst.GpuRequest != 2 {
		t.Fatalf("GpuRequest = %+v, want 2", dst.GpuRequest)
	}
	if dst.GpuLimit == nil || *dst.GpuLimit != 3 {
		t.Fatalf("GpuLimit = %+v, want 3", dst.GpuLimit)
	}
	if dst.GPUNumDevices == nil || *dst.GPUNumDevices != 4 {
		t.Fatalf("GPUNumDevices = %+v, want 4", dst.GPUNumDevices)
	}
}

func testStoreTracksRestartedRelevantPod(t *testing.T, exitCode int32) {
	pod := newKaiFractionPod("kai-restart-pod", "5555555555555555555555555555555555555555555555555555555555555555", 0.625)
	store, client, stop := startTestStore(t, pod)
	defer stop()

	oldKey := &model.GpuContainerKey{
		PodUid:      pod.UID,
		ContainerId: "5555555555555555555555555555555555555555555555555555555555555555",
	}
	newKey := &model.GpuContainerKey{
		PodUid:      pod.UID,
		ContainerId: "6666666666666666666666666666666666666666666666666666666666666666",
	}
	waitForGPUContainerTotal(t, store, 1)

	restarted := pod.DeepCopy()
	restarted.Status.ContainerStatuses = []corev1.ContainerStatus{
		{
			Name:        mainContainerName,
			ContainerID: withRuntimePrefix(newKey.ContainerId),
			State: corev1.ContainerState{
				Running: &corev1.ContainerStateRunning{StartedAt: metav1.NewTime(time.Now())},
			},
			LastTerminationState: corev1.ContainerState{
				Terminated: &corev1.ContainerStateTerminated{
					ContainerID: withRuntimePrefix(oldKey.ContainerId),
					ExitCode:    exitCode,
					FinishedAt:  metav1.NewTime(time.Now().Add(-1 * time.Minute)),
				},
			},
		},
	}
	if _, err := client.CoreV1().Pods(restarted.Namespace).UpdateStatus(
		t.Context(),
		restarted,
		metav1.UpdateOptions{},
	); err != nil {
		t.Fatalf("update pod status: %v", err)
	}

	waitForGPUContainerTotal(t, store, 2)
	waitForContainerState(t, store, oldKey, func(gc *model.GpuContainer) bool {
		return !gc.FinishedAt.IsZero()
	})
	waitForContainerState(t, store, newKey, func(gc *model.GpuContainer) bool {
		return gc.FinishedAt.IsZero()
	})
}

func startTestStore(t *testing.T, objects ...runtime.Object) (*Store, *kubefake.Clientset, func()) {
	t.Helper()

	client := kubefake.NewSimpleClientset(objects...)
	store := &Store{
		client:        client,
		stopCh:        make(chan struct{}),
		cfg:           &config.Config{NodeName: testNodeName, ScrapeInterval: time.Second},
		gpuContainers: make(map[types.UID]map[string]*model.GpuContainer),
	}
	store.cleanupScheduler = NewCleanupScheduler(store)

	factory := informers.NewSharedInformerFactoryWithOptions(
		client,
		0,
		informers.WithTweakListOptions(func(options *metav1.ListOptions) {
			options.FieldSelector = "spec.nodeName=" + testNodeName
		}),
	)

	store.informer = factory.Core().V1().Pods().Informer()
	if _, err := store.informer.AddEventHandler(cache.ResourceEventHandlerFuncs{
		AddFunc: func(obj interface{}) {
			store.reconcilePod(obj)
		},
		UpdateFunc: func(_, newObj interface{}) {
			store.reconcilePod(newObj)
		},
		DeleteFunc: func(obj interface{}) {
			store.finishDeletedPod(obj)
		},
	}); err != nil {
		t.Fatalf("add event handler: %v", err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		select {
		case <-ctx.Done():
			store.Stop()
		case <-store.stopCh:
		}
	}()
	go store.informer.Run(store.stopCh)

	if !cache.WaitForCacheSync(ctx.Done(), store.informer.HasSynced) {
		cancel()
		t.Fatalf("cache sync failed")
	}

	return store, client, func() {
		cancel()
		store.Stop()
	}
}

func waitForGPUContainerTotal(t *testing.T, store *Store, want int) {
	t.Helper()
	waitForCondition(t, fmt.Sprintf("gpu container total == %d", want), func() bool {
		return countGPUContainers(store) == want
	})
}

func waitForContainerState(
	t *testing.T,
	store *Store,
	key *model.GpuContainerKey,
	predicate func(*model.GpuContainer) bool,
) {
	t.Helper()
	waitForCondition(t, fmt.Sprintf("container %s/%s state", key.PodUid, key.ContainerId), func() bool {
		store.gpuContainersMu.RLock()
		defer store.gpuContainersMu.RUnlock()
		gc, ok := store.gpuContainers[key.PodUid][key.ContainerId]
		return ok && predicate(gc)
	})
}

func assertContainer(t *testing.T, store *Store, key *model.GpuContainerKey, check func(*model.GpuContainer)) {
	t.Helper()
	waitForCondition(t, fmt.Sprintf("container %s/%s exists", key.PodUid, key.ContainerId), func() bool {
		_, ok := store.get(key)
		return ok
	})

	store.gpuContainersMu.RLock()
	defer store.gpuContainersMu.RUnlock()
	gc := store.gpuContainers[key.PodUid][key.ContainerId]
	if gc == nil {
		t.Fatalf("missing gpu container for key %+v", key)
	}
	if check != nil {
		check(gc)
	}
}

func waitForCondition(t *testing.T, desc string, fn func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if fn() {
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %s", desc)
}

func countGPUContainers(store *Store) int {
	store.gpuContainersMu.RLock()
	defer store.gpuContainersMu.RUnlock()

	total := 0
	for _, byID := range store.gpuContainers {
		total += len(byID)
	}
	return total
}

func waitForCleanupSchedulerSize(t *testing.T, store *Store, want int) {
	t.Helper()
	waitForCondition(t, fmt.Sprintf("cleanup scheduler size == %d", want), func() bool {
		return countCleanupSchedulerEntries(store) == want
	})
}

func countCleanupSchedulerEntries(store *Store) int {
	store.gpuContainersMu.RLock()
	defer store.gpuContainersMu.RUnlock()

	if store.cleanupScheduler == nil {
		return 0
	}
	return len(store.cleanupScheduler.queued)
}

func newStoreTestGpuMetricsProvider() *gpuprom.GpuMetricsProvider {
	perDeviceLabels := []string{
		gpuprom.LabelNode,
		gpuprom.LabelNamespace,
		gpuprom.LabelPod,
		gpuprom.LabelContainer,
		gpuprom.LabelContainerId,
		gpuprom.LabelGpuAllocationType,
		gpuprom.LabelGpuUuid,
		gpuprom.LabelGpuModel,
	}
	return &gpuprom.GpuMetricsProvider{
		MemoryBytes:              newStoreTestGaugeVec("test_store_zero_memory_bytes", perDeviceLabels),
		MemoryFootprint:          newStoreTestGaugeVec("test_store_zero_memory_footprint", perDeviceLabels),
		ProtectedMemoryBytes:     newStoreTestGaugeVec("test_store_zero_protected_memory_bytes", perDeviceLabels),
		AccountingGpuUtil:        newStoreTestGaugeVec("test_store_zero_accounting_gpu_util", perDeviceLabels),
		AccountingMemUtil:        newStoreTestGaugeVec("test_store_zero_accounting_mem_util", perDeviceLabels),
		AccountingMaxMemoryBytes: newStoreTestGaugeVec("test_store_zero_accounting_max_memory_bytes", perDeviceLabels),
		AccountingIsRunning:      newStoreTestGaugeVec("test_store_zero_accounting_is_running", perDeviceLabels),
	}
}

func newStoreTestGaugeVec(name string, labels []string) *clientprom.GaugeVec {
	return clientprom.NewGaugeVec(clientprom.GaugeOpts{Name: name, Help: testMetricHelpStore}, labels)
}

func storeTestGaugeValue(t *testing.T, metric clientprom.Metric) float64 {
	t.Helper()

	var m dto.Metric
	if err := metric.Write(&m); err != nil {
		t.Fatalf("write metric: %v", err)
	}
	if m.Gauge == nil {
		t.Fatalf("metric is not a gauge")
	}
	return m.GetGauge().GetValue()
}

func newKaiReservationPod(name, containerName, containerID string) *corev1.Pod {
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: KaiResourceReservationNamespace,
			UID:       types.UID(name + "-uid"),
		},
		Spec: corev1.PodSpec{
			NodeName: testNodeName,
			Containers: []corev1.Container{
				{
					Name: containerName,
					Resources: corev1.ResourceRequirements{
						Requests: corev1.ResourceList{NvidiaGpuResource: resource.MustParse("1")},
						Limits:   corev1.ResourceList{NvidiaGpuResource: resource.MustParse("1")},
					},
				},
			},
		},
		Status: corev1.PodStatus{
			ContainerStatuses: []corev1.ContainerStatus{
				runningStatus(containerName, containerID),
			},
		},
	}
}

func newKaiFractionPod(name, containerID string, fraction float64) *corev1.Pod {
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: testNamespace,
			UID:       types.UID(name + "-uid"),
			Annotations: map[string]string{
				KaiFractionAnnotation: fmt.Sprintf("%g", fraction),
			},
		},
		Spec: corev1.PodSpec{
			NodeName: testNodeName,
			Containers: []corev1.Container{
				{Name: mainContainerName},
			},
		},
		Status: corev1.PodStatus{
			ContainerStatuses: []corev1.ContainerStatus{
				runningStatus(mainContainerName, containerID),
			},
		},
	}
}

func newKaiMemoryPod(name, containerName, containerID string, memory int64) *corev1.Pod {
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: testNamespace,
			UID:       types.UID(name + "-uid"),
			Annotations: map[string]string{
				KaiMemoryAnnotation:            fmt.Sprintf("%d", memory),
				KaiFractionContainerAnnotation: containerName,
			},
		},
		Spec: corev1.PodSpec{
			NodeName: testNodeName,
			Containers: []corev1.Container{
				{Name: "sidecar"},
				{Name: containerName},
			},
		},
		Status: corev1.PodStatus{
			ContainerStatuses: []corev1.ContainerStatus{
				runningStatus("sidecar", "7777777777777777777777777777777777777777777777777777777777777777"),
				runningStatus(containerName, containerID),
			},
		},
	}
}

func newKaiInitContainerPod(name, containerName, containerID string, fraction float64) *corev1.Pod {
	restartPolicy := corev1.ContainerRestartPolicyAlways
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: testNamespace,
			UID:       types.UID(name + "-uid"),
			Annotations: map[string]string{
				KaiFractionAnnotation:          fmt.Sprintf("%g", fraction),
				KaiFractionContainerAnnotation: containerName,
			},
		},
		Spec: corev1.PodSpec{
			NodeName: testNodeName,
			Containers: []corev1.Container{
				{Name: mainContainerName},
			},
			InitContainers: []corev1.Container{
				{
					Name:          containerName,
					RestartPolicy: &restartPolicy,
				},
			},
		},
		Status: corev1.PodStatus{
			ContainerStatuses: []corev1.ContainerStatus{
				runningStatus(mainContainerName, "8888888888888888888888888888888888888888888888888888888888888888"),
			},
			InitContainerStatuses: []corev1.ContainerStatus{
				runningStatus(containerName, containerID),
			},
		},
	}
}

func newStandardGPUPod(name, containerName, containerID string) *corev1.Pod {
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: testNamespace,
			UID:       types.UID(name + "-uid"),
		},
		Spec: corev1.PodSpec{
			NodeName: testNodeName,
			Containers: []corev1.Container{
				{
					Name: containerName,
					Resources: corev1.ResourceRequirements{
						Requests: corev1.ResourceList{NvidiaGpuResource: resource.MustParse("1")},
						Limits:   corev1.ResourceList{NvidiaGpuResource: resource.MustParse("1")},
					},
				},
			},
		},
		Status: corev1.PodStatus{
			ContainerStatuses: []corev1.ContainerStatus{
				runningStatus(containerName, containerID),
			},
		},
	}
}

func newIrrelevantPod(name string) *corev1.Pod {
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: testNamespace,
			UID:       types.UID(name + "-uid"),
		},
		Spec: corev1.PodSpec{
			NodeName: testNodeName,
			Containers: []corev1.Container{
				{Name: mainContainerName},
			},
		},
		Status: corev1.PodStatus{
			ContainerStatuses: []corev1.ContainerStatus{
				runningStatus(mainContainerName, "9999999999999999999999999999999999999999999999999999999999999999"),
			},
		},
	}
}

func runningStatus(name, containerID string) corev1.ContainerStatus {
	return corev1.ContainerStatus{
		Name:        name,
		ContainerID: withRuntimePrefix(containerID),
		State: corev1.ContainerState{
			Running: &corev1.ContainerStateRunning{
				StartedAt: metav1.NewTime(time.Now()),
			},
		},
	}
}

func terminatedStatus(name, containerID string) corev1.ContainerStatus {
	return corev1.ContainerStatus{
		Name:        name,
		ContainerID: withRuntimePrefix(containerID),
		State: corev1.ContainerState{
			Terminated: &corev1.ContainerStateTerminated{
				ContainerID: withRuntimePrefix(containerID),
				FinishedAt:  metav1.NewTime(time.Now()),
			},
		},
	}
}

func withRuntimePrefix(containerID string) string {
	return "containerd://" + containerID
}
