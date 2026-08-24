// SPDX-License-Identifier: Apache-2.0

// Package kube provides the store which uses list-and-watch paradigm for this node's pods and
// takes care of tracking the containers using Nvidia's GPUs.
package kube

import (
	"context"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	"github.com/densify-dev/gpu-process-exporter/pkg/process"
	"github.com/densify-dev/gpu-process-exporter/pkg/prometheus"
	"github.com/densify-dev/gpu-process-exporter/pkg/value"
	authorizationv1 "k8s.io/api/authorization/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/fields"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/informers"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/cache"
	"k8s.io/client-go/tools/clientcmd"
)

const (
	NvidiaGpuResource corev1.ResourceName = "nvidia.com/gpu"
)
const (
	KaiResourceReservationNamespace  = "kai-resource-reservation"
	KaiResourceReservationAnnotation = "run.ai/reserve_for_gpu_index"
	KaiFractionAnnotation            = "gpu-fraction"
	KaiMemoryAnnotation              = "gpu-memory"
	KaiFractionContainerAnnotation   = "gpu-fraction-container-name"
	KaiNumDevicesAnnotation          = "gpu-fraction-num-devices"
	nodeNameKey                      = "spec.nodeName"
)

// +kubebuilder:rbac:groups="",resources=pods,verbs=get;list;watch

type Store struct {
	client           kubernetes.Interface
	informer         cache.SharedIndexInformer
	stopCh           chan struct{}
	stopOnce         sync.Once
	cfg              *config.Config
	gpuContainersMu  sync.RWMutex
	gpuContainers    map[types.UID]map[string]*model.GpuContainer
	cleanupScheduler *CleanupScheduler
	mapper           *process.Mapper
	gmp              *prometheus.GpuMetricsProvider
}

func NewStore(
	ctx context.Context,
	cfg *config.Config,
	gmp *prometheus.GpuMetricsProvider,
) (s *Store, err error) {
	var restConfig *rest.Config
	if restConfig, err = newRestConfig(); err == nil {
		s, err = NewStoreFromConfig(ctx, restConfig, cfg, gmp)
	}
	return
}

func newRestConfig() (restConfig *rest.Config, err error) {
	if restConfig, err = clientcmd.NewNonInteractiveDeferredLoadingClientConfig(
		clientcmd.NewDefaultClientConfigLoadingRules(),
		&clientcmd.ConfigOverrides{},
	).ClientConfig(); err == nil {
		return
	}
	log.Printf("kube: could not load kubeconfig from filesystem, falling back to in-cluster config")
	if restConfig, err = rest.InClusterConfig(); err != nil {
		err = fmt.Errorf("create in-cluster config: %w", err)
	}
	return
}

func NewStoreFromConfig(
	ctx context.Context,
	restConfig *rest.Config,
	cfg *config.Config,
	gmp *prometheus.GpuMetricsProvider,
) (*Store, error) {
	client, err := kubernetes.NewForConfig(restConfig)
	if err != nil {
		return nil, fmt.Errorf("create kubernetes client: %w", err)
	}
	if err = verifyPodAccess(ctx, client); err != nil {
		return nil, err
	}

	store := &Store{
		client:        client,
		stopCh:        make(chan struct{}),
		cfg:           cfg,
		gpuContainers: make(map[types.UID]map[string]*model.GpuContainer),
		mapper:        process.NewMapper(cfg),
		gmp:           gmp,
	}
	store.cleanupScheduler = NewCleanupScheduler(store)

	factory := informers.NewSharedInformerFactoryWithOptions(
		client,
		0,
		informers.WithTweakListOptions(func(options *metav1.ListOptions) {
			options.FieldSelector = fields.OneTermEqualSelector(nodeNameKey, store.cfg.NodeName).String()
		}),
	)

	store.informer = factory.Core().V1().Pods().Informer()
	_, err = store.informer.AddEventHandler(cache.ResourceEventHandlerFuncs{
		AddFunc: func(obj interface{}) {
			store.reconcilePod(obj)
		},
		UpdateFunc: func(_, newObj interface{}) {
			store.reconcilePod(newObj)
		},
		DeleteFunc: func(obj interface{}) {
			store.finishDeletedPod(obj)
		},
	})
	if err != nil {
		return nil, fmt.Errorf("register pod event handler: %w", err)
	}

	go func() {
		select {
		case <-ctx.Done():
			store.Stop()
		case <-store.stopCh:
		}
	}()

	go store.informer.Run(store.stopCh)

	if !cache.WaitForCacheSync(ctx.Done(), store.informer.HasSynced) {
		store.Stop()
		if err = ctx.Err(); err != nil {
			return nil, fmt.Errorf("sync pod informer: %w", err)
		}
		return nil, fmt.Errorf("sync pod informer: cache sync failed")
	}

	go store.cleanupScheduler.StartSweeper(ctx)

	return store, nil
}

func (s *Store) Stop() {
	s.stopOnce.Do(func() {
		close(s.stopCh)
	})
}

func (s *Store) GetMapper() *process.Mapper {
	return s.mapper
}

//nolint:unparam
func (s *Store) get(gck *model.GpuContainerKey) (gc *model.GpuContainer, ok bool) {
	if s == nil || gck == nil {
		return
	}
	s.gpuContainersMu.RLock()
	defer s.gpuContainersMu.RUnlock()
	gc, ok = s.gpuContainers[gck.PodUid][gck.ContainerId]
	return
}

func (s *Store) GetMetricLabelsAndValues(
	gck *model.GpuContainerKey,
	devInfo *model.DeviceInfo,
) (
	containerLabels []string,
	perDeviceLabels []string,
	deviceLabelsFingerprint uint64,
	calculatedValues *model.CalculatedValues,
	ok bool,
) {
	if s == nil || gck == nil || devInfo == nil {
		return
	}
	s.gpuContainersMu.Lock()
	defer s.gpuContainersMu.Unlock()
	gc := s.gpuContainers[gck.PodUid][gck.ContainerId]
	if gc == nil {
		return
	}
	containerLabels = gc.GetContainerLabels()
	dmf := gc.DeviceLabels[devInfo.Uuid]
	if dmf == nil {
		dmf = &model.DeviceModelFingerprint{}
		gc.DeviceLabels[devInfo.Uuid] = dmf
	}
	dmf.ConditionallySet(devInfo.Uuid, devInfo.ModelName, containerLabels)
	perDeviceLabels, deviceLabelsFingerprint = dmf.PerDeviceLabels, dmf.Fingerprint
	calculatedValues = gc.GetCalculatedValues(devInfo)
	ok = true
	return
}

func verifyPodAccess(ctx context.Context, client kubernetes.Interface) error {
	for _, verb := range []string{"get", "list", "watch"} {
		review, err := client.AuthorizationV1().SelfSubjectAccessReviews().Create(
			ctx,
			&authorizationv1.SelfSubjectAccessReview{
				Spec: authorizationv1.SelfSubjectAccessReviewSpec{
					ResourceAttributes: &authorizationv1.ResourceAttributes{
						Group:    "",
						Resource: "pods",
						Verb:     verb,
					},
				},
			},
			metav1.CreateOptions{},
		)
		if err != nil {
			return fmt.Errorf("check %s permission for pods: %w", verb, err)
		}
		if !review.Status.Allowed {
			reason := strings.TrimSpace(review.Status.Reason)
			if reason == "" {
				reason = "access denied"
			}
			return fmt.Errorf("missing %s permission for pods: %s", verb, reason)
		}
	}
	return nil
}

func (s *Store) reconcilePod(obj interface{}) {
	pod, ok := podFromObject(obj)
	if !ok || pod == nil {
		return
	}

	desired := desiredGpuContainersForPod(pod)

	s.gpuContainersMu.Lock()
	defer s.gpuContainersMu.Unlock()

	currentByID := make(map[string]*model.GpuContainer, len(desired))
	for _, container := range desired {
		currentByID[container.ContainerId] = container
	}

	existingByID := s.gpuContainers[pod.UID]
	for _, existing := range existingByID {
		if _, ok := currentByID[existing.ContainerId]; ok {
			continue
		}
		if existing.FinishedAt.IsZero() {
			s.finishContainerAt(existing, finishedAtForContainer(pod, existing))
		}
	}

	for _, container := range desired {
		if existing := existingByID[container.ContainerId]; existing != nil {
			updateGpuContainer(existing, container)
			if existing.FinishedAt.IsZero() {
				s.finishContainerAt(existing, observedFinishedAtForContainer(pod, existing))
			}
			continue
		}
		container.FinishedAt = observedFinishedAtForContainer(pod, container)
		if existingByID == nil {
			existingByID = make(map[string]*model.GpuContainer)
			s.gpuContainers[pod.UID] = existingByID
		}
		existingByID[container.ContainerId] = container
		s.stageForCleanup(container)
	}
}

func (s *Store) finishDeletedPod(obj interface{}) {
	pod, ok := podFromObject(obj)
	if !ok || pod == nil {
		return
	}

	s.gpuContainersMu.Lock()
	defer s.gpuContainersMu.Unlock()

	existingByID := s.gpuContainers[pod.UID]
	for _, container := range existingByID {
		if container.FinishedAt.IsZero() {
			finishedAt := finishedAtForContainer(pod, container)
			if finishedAt.IsZero() {
				finishedAt = time.Now()
			}
			s.finishContainerAt(container, finishedAt)
		}
	}
}

func (s *Store) finishContainerAt(container *model.GpuContainer, finishedAt time.Time) {
	// this function is called ONLY when s.gpuContainersMu is locked
	if container == nil || finishedAt.IsZero() {
		return
	}
	container.FinishedAt = finishedAt
	if s.gmp != nil {
		s.gmp.ZeroValues(ensureAllPerDeviceLabelFingerprints(container))
	}
	s.stageForCleanup(container)
}

func ensureAllPerDeviceLabelFingerprints(container *model.GpuContainer) []model.LabelFingerprint {
	if container == nil {
		return nil
	}
	labelFingerprints := make([]model.LabelFingerprint, 0, len(container.DeviceLabels))
	for deviceKey, device := range container.DeviceLabels {
		if device == nil {
			continue
		}
		device.ConditionallySet(deviceKey, device.ModelName, container.GetContainerLabels())
		labels, fingerprint := container.GetPerDeviceLabelsFingerprint(deviceKey)
		labelFingerprints = append(labelFingerprints, model.LabelFingerprint{
			Labels:      labels,
			Fingerprint: fingerprint,
		})
	}
	return labelFingerprints
}

func (s *Store) stageForCleanup(container *model.GpuContainer) {
	// this function is called ONLY when s.gpuContainersMu is locked
	s.cleanupScheduler.enqueue(container)
}

func desiredGpuContainersForPod(pod *corev1.Pod) map[string]*model.GpuContainer {
	desired := make(map[string]*model.GpuContainer)
	addK8sResourceContainers(pod, desired)
	addKaiContainers(pod, desired)
	return desired
}

func addK8sResourceContainers(pod *corev1.Pod, desired map[string]*model.GpuContainer) {
	if pod.Namespace == KaiResourceReservationNamespace {
		return
	}
	if _, reserved := ParseAnnotation[string](&pod.ObjectMeta, KaiResourceReservationAnnotation); reserved {
		return
	}

	for _, container := range pod.Spec.Containers {
		addK8sContainerIfGPURequested(pod, container, false, desired)
	}
	for _, container := range pod.Spec.InitContainers {
		addK8sContainerIfGPURequested(pod, container, true, desired)
	}
}

func addK8sContainerIfGPURequested(
	pod *corev1.Pod,
	container corev1.Container,
	initContainer bool,
	desired map[string]*model.GpuContainer,
) {
	request, hasRequest := container.Resources.Requests[NvidiaGpuResource]
	limit, hasLimit := container.Resources.Limits[NvidiaGpuResource]
	if !hasRequest && !hasLimit {
		return
	}

	status, ok := findContainerStatus(pod, container.Name, initContainer)
	if !ok {
		return
	}

	containerID := normalizeContainerID(status.ContainerID)
	if containerID == value.Empty {
		return
	}

	var req, lim *int64
	if hasRequest {
		if n, ok := request.AsInt64(); ok {
			req = &n
		}
	}
	if hasLimit {
		if n, ok := limit.AsInt64(); ok {
			lim = &n
		}
	}

	desired[containerID] = newGpuContainer(
		pod,
		container.Name,
		containerID,
		model.K8sResource,
		func(gc *model.GpuContainer) {
			gc.GpuRequest = req
			gc.GpuLimit = lim
		},
	)
}

func addKaiContainers(pod *corev1.Pod, desired map[string]*model.GpuContainer) {
	fraction, hasFraction := ParseAnnotation[float64](&pod.ObjectMeta, KaiFractionAnnotation)
	memory, hasMemory := ParseAnnotation[uint64](&pod.ObjectMeta, KaiMemoryAnnotation)
	numDevices, hasNumDevices := ParseAnnotation[uint64](&pod.ObjectMeta, KaiNumDevicesAnnotation)
	if !hasFraction && !hasMemory && !hasNumDevices {
		return
	}

	container, initContainer, ok := selectKaiContainer(pod)
	if !ok {
		return
	}

	status, ok := findContainerStatus(pod, container.Name, initContainer)
	if !ok {
		return
	}

	containerID := normalizeContainerID(status.ContainerID)
	if containerID == "" {
		return
	}

	desired[containerID] = newGpuContainer(
		pod,
		container.Name,
		containerID,
		model.KaiScheduler,
		func(gc *model.GpuContainer) {
			if hasFraction {
				gc.GpuFraction = new(fraction)
			}
			if hasMemory {
				gc.GpuMemory = new(memory)
			}
			if hasNumDevices {
				gc.GPUNumDevices = new(numDevices)
			}
		},
	)
}

func selectKaiContainer(pod *corev1.Pod) (corev1.Container, bool, bool) {
	if name, ok := pod.Annotations[KaiFractionContainerAnnotation]; ok && strings.TrimSpace(name) != "" {
		for _, container := range pod.Spec.Containers {
			if container.Name == name {
				return container, false, true
			}
		}
		for _, container := range pod.Spec.InitContainers {
			if container.Name != name {
				continue
			}
			if container.RestartPolicy == nil || *container.RestartPolicy != corev1.ContainerRestartPolicyAlways {
				return corev1.Container{}, false, false
			}
			return container, true, true
		}
		return corev1.Container{}, false, false
	}

	if len(pod.Spec.Containers) == 0 {
		return corev1.Container{}, false, false
	}
	return pod.Spec.Containers[0], false, true
}

func findContainerStatus(pod *corev1.Pod, name string, initContainer bool) (*corev1.ContainerStatus, bool) {
	var statuses []corev1.ContainerStatus
	if initContainer {
		statuses = pod.Status.InitContainerStatuses
	} else {
		statuses = pod.Status.ContainerStatuses
	}
	for _, status := range statuses {
		if status.Name == name {
			return &status, true
		}
	}
	return nil, false
}

func newGpuContainer(
	pod *corev1.Pod,
	containerName, containerID string,
	allocationType model.GpuAllocationType,
	mutate func(*model.GpuContainer),
) *model.GpuContainer {
	key := &model.GpuContainerKey{
		ContainerId: containerID,
		PodUid:      pod.UID,
	}

	container := &model.GpuContainer{
		NodeName:          pod.Spec.NodeName,
		GpuContainerKey:   key,
		Namespace:         pod.Namespace,
		PodName:           pod.Name,
		ContainerName:     containerName,
		GpuAllocationType: allocationType,
		DeviceLabels:      make(map[string]*model.DeviceModelFingerprint),
	}

	if mutate != nil {
		mutate(container)
	}

	return container
}

func updateGpuContainer(dst, src *model.GpuContainer) {
	labelsChanged := dst.NodeName != src.NodeName ||
		dst.Namespace != src.Namespace ||
		dst.PodName != src.PodName ||
		dst.ContainerName != src.ContainerName
	dst.NodeName = src.NodeName
	dst.Namespace = src.Namespace
	dst.PodName = src.PodName
	dst.ContainerName = src.ContainerName
	dst.GpuAllocationType = src.GpuAllocationType
	dst.GpuUuid = src.GpuUuid
	dst.GpuRequest = src.GpuRequest
	dst.GpuLimit = src.GpuLimit
	dst.GpuFraction = src.GpuFraction
	dst.GpuMemory = src.GpuMemory
	dst.GPUNumDevices = src.GPUNumDevices
	dst.ResetCalculatedValues()
	if labelsChanged {
		dst.ResetDeviceLabelCache()
	}
}

func finishedAtForContainer(pod *corev1.Pod, container *model.GpuContainer) time.Time {
	if finishedAt := observedFinishedAtForContainer(pod, container); !finishedAt.IsZero() {
		return finishedAt
	}
	if pod.DeletionTimestamp != nil && !pod.DeletionTimestamp.IsZero() {
		return pod.DeletionTimestamp.Time
	}
	if pod.Status.Phase == corev1.PodFailed || pod.Status.Phase == corev1.PodSucceeded {
		return time.Now()
	}
	return time.Time{}
}

func observedFinishedAtForContainer(pod *corev1.Pod, container *model.GpuContainer) time.Time {
	for _, status := range append([]corev1.ContainerStatus{}, pod.Status.ContainerStatuses...) {
		if finishedAt := finishedAtFromStatus(status, container); !finishedAt.IsZero() {
			return finishedAt
		}
	}
	for _, status := range append([]corev1.ContainerStatus{}, pod.Status.InitContainerStatuses...) {
		if finishedAt := finishedAtFromStatus(status, container); !finishedAt.IsZero() {
			return finishedAt
		}
	}
	return time.Time{}
}

func finishedAtFromStatus(status corev1.ContainerStatus, container *model.GpuContainer) time.Time {
	if status.Name != container.ContainerName {
		return time.Time{}
	}

	if terminated := status.State.Terminated; terminated != nil &&
		normalizeContainerID(terminated.ContainerID) == container.ContainerId {
		return terminated.FinishedAt.Time
	}
	if terminated := status.LastTerminationState.Terminated; terminated != nil &&
		normalizeContainerID(terminated.ContainerID) == container.ContainerId {
		return terminated.FinishedAt.Time
	}
	return time.Time{}
}

func normalizeContainerID(containerID string) string {
	if containerID == value.Empty {
		return value.Empty
	}
	if parts := strings.SplitN(containerID, "://", 2); len(parts) == 2 {
		return parts[1]
	}
	return containerID
}

func podFromObject(obj interface{}) (*corev1.Pod, bool) {
	switch val := obj.(type) {
	case *corev1.Pod:
		return val, true
	case cache.DeletedFinalStateUnknown:
		pod, ok := val.Obj.(*corev1.Pod)
		return pod, ok
	case *cache.DeletedFinalStateUnknown:
		if val == nil {
			return nil, false
		}
		pod, ok := val.Obj.(*corev1.Pod)
		return pod, ok
	default:
		return nil, false
	}
}

func ParseAnnotation[T value.Value](objMeta *metav1.ObjectMeta, key string) (t T, ok bool) {
	if objMeta != nil {
		t, ok = value.FindParsedValue[T](value.MapProvider(objMeta.Annotations), key)
	}
	return
}
