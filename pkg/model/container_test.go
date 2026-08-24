// SPDX-License-Identifier: Apache-2.0

package model

import (
	"sync"
	"testing"

	"github.com/densify-dev/gpu-process-exporter/pkg/value"
	"k8s.io/apimachinery/pkg/types"
)

const (
	testContainerID   = "container-id"
	testNodeName      = "node-a"
	testNamespace     = "ns-a"
	testPodName       = "pod-a"
	testContainerName = "ctr-a"
	testGpuUUID       = "gpu-a"
	testModelName     = "model-a"
)

func TestGpuAllocationTypeString(t *testing.T) {
	if got := K8sResource.String(); got != "K8sResource" {
		t.Fatalf("K8sResource.String() = %q, want %q", got, "K8sResource")
	}
	if got := KaiScheduler.String(); got != "KaiScheduler" {
		t.Fatalf("KaiScheduler.String() = %q, want %q", got, "KaiScheduler")
	}
}

func TestGpuContainerKeyString(t *testing.T) {
	key := &GpuContainerKey{PodUid: types.UID("pod-uid"), ContainerId: testContainerID}
	if got := key.String(); got != "pod-uid/container-id" {
		t.Fatalf("GpuContainerKey.String() = %q, want %q", got, "pod-uid/container-id")
	}

	var nilKey *GpuContainerKey
	if got := nilKey.String(); got != "" {
		t.Fatalf("nil GpuContainerKey.String() = %q, want empty string", got)
	}
}

func TestGpuContainerLabelHelpers(t *testing.T) {
	gc := &GpuContainer{
		GpuContainerKey: &GpuContainerKey{ContainerId: testContainerID},
		NodeName:        testNodeName,
		Namespace:       testNamespace,
		PodName:         testPodName,
		ContainerName:   testContainerName,
		DeviceLabels: map[string]*DeviceModelFingerprint{
			testGpuUUID: {ModelName: testModelName},
			"gpu-b":     {ModelName: "model-b"},
		},
	}

	containerLabels := gc.GetContainerLabels()
	wantContainerLabels := []string{
		testNodeName,
		testNamespace,
		testPodName,
		testContainerName,
		testContainerID,
		K8sResource.String(),
	}
	if !equalStrings(containerLabels, wantContainerLabels) {
		t.Fatalf("GetContainerLabels() = %v, want %v", containerLabels, wantContainerLabels)
	}

	perDeviceLabels := gc.GetPerDeviceLabels(testGpuUUID)
	wantPerDeviceLabels := []string{
		testNodeName,
		testNamespace,
		testPodName,
		testContainerName,
		testContainerID,
		K8sResource.String(),
		testGpuUUID,
		testModelName,
	}
	if !equalStrings(perDeviceLabels, wantPerDeviceLabels) {
		t.Fatalf("GetPerDeviceLabels() = %v, want %v", perDeviceLabels, wantPerDeviceLabels)
	}
	gc.DeviceLabels[testGpuUUID].ConditionallySet(testGpuUUID, testModelName, containerLabels)
	if !equalStrings(gc.DeviceLabels[testGpuUUID].PerDeviceLabels, wantPerDeviceLabels) {
		t.Fatalf(
			"ConditionallySet() after GetPerDeviceLabels() = %v, want %v",
			gc.DeviceLabels[testGpuUUID].PerDeviceLabels,
			wantPerDeviceLabels,
		)
	}
	if got := gc.DeviceLabels[testGpuUUID].Fingerprint; got != value.Fingerprint(wantPerDeviceLabels) {
		t.Fatalf("Fingerprint = %d, want %d", got, value.Fingerprint(wantPerDeviceLabels))
	}

	all := gc.GetAllPerDeviceLabels()
	if len(all) != 2 {
		t.Fatalf("len(GetAllPerDeviceLabels()) = %d, want 2", len(all))
	}
}

func TestGpuContainerCalculateK8sResource(t *testing.T) {
	request := int64(2)
	limit := int64(3)
	gc := &GpuContainer{
		GpuRequest: &request,
		GpuLimit:   &limit,
	}
	devInfo := &DeviceInfo{TotalMemory: 16 * MiB}

	requests, limits, totalMemory, err := gc.calculate(devInfo)
	if err != nil {
		t.Fatalf("calculate() error = %v, want nil", err)
	}
	if requests != 2 || limits != 3 {
		t.Fatalf("calculate() requests/limits = %v/%v, want 2/3", requests, limits)
	}
	if totalMemory != float64(16*MiB) {
		t.Fatalf("calculate() totalMemory = %v, want %d", totalMemory, 16*MiB)
	}
}

func TestGpuContainerCalculateKaiFraction(t *testing.T) {
	fraction := 0.25
	numDevices := uint64(2)
	gc := &GpuContainer{
		GpuFraction:   &fraction,
		GPUNumDevices: &numDevices,
	}
	devInfo := &DeviceInfo{TotalMemory: 16 * MiB}

	requests, limits, totalMemory, err := gc.calculate(devInfo)
	if err != nil {
		t.Fatalf("calculate() error = %v, want nil", err)
	}
	if requests != 0.5 || limits != 0.5 {
		t.Fatalf("calculate() requests/limits = %v/%v, want 0.5/0.5", requests, limits)
	}
	if totalMemory != float64(4*MiB) {
		t.Fatalf("calculate() totalMemory = %v, want %d", totalMemory, 4*MiB)
	}
}

func TestGpuContainerCalculateKaiMemory(t *testing.T) {
	memory := uint64(4096)
	numDevices := uint64(2)
	gc := &GpuContainer{
		GpuMemory:     &memory,
		GPUNumDevices: &numDevices,
	}
	devInfo := &DeviceInfo{TotalMemory: 16384 * MiB}

	requests, limits, totalMemory, err := gc.calculate(devInfo)
	if err != nil {
		t.Fatalf("calculate() error = %v, want nil", err)
	}
	if requests != 0.5 || limits != 0.5 {
		t.Fatalf("calculate() requests/limits = %v/%v, want 0.5/0.5", requests, limits)
	}
	if totalMemory != float64(4096*MiB) {
		t.Fatalf("calculate() totalMemory = %v, want %d", totalMemory, 4096*MiB)
	}
}

func TestGpuContainerCalculateErrors(t *testing.T) {
	if _, _, _, err := (*GpuContainer)(nil).calculate(&DeviceInfo{TotalMemory: 1}); err == nil {
		t.Fatal("nil GpuContainer calculate() error = nil, want error")
	}
	if _, _, _, err := (&GpuContainer{}).calculate(nil); err == nil {
		t.Fatal("nil DeviceInfo calculate() error = nil, want error")
	}
	if _, _, _, err := (&GpuContainer{}).calculate(&DeviceInfo{}); err == nil {
		t.Fatal("zero total memory calculate() error = nil, want error")
	}
	if _, _, _, err := (&GpuContainer{}).calculate(&DeviceInfo{TotalMemory: 1}); err == nil {
		t.Fatal("missing allocation calculate() error = nil, want error")
	}
}

func TestGetCalculatedValuesCachesFirstResult(t *testing.T) {
	fraction := 0.5
	gc := &GpuContainer{GpuFraction: &fraction}

	first := gc.GetCalculatedValues(&DeviceInfo{TotalMemory: 8 * MiB})
	if first == nil {
		t.Fatal("first GetCalculatedValues() = nil, want value")
	}
	second := gc.GetCalculatedValues(&DeviceInfo{TotalMemory: 16 * MiB})
	if second == nil {
		t.Fatal("second GetCalculatedValues() = nil, want cached value")
	}
	if first != second {
		t.Fatal("GetCalculatedValues() did not return cached pointer")
	}
	if second.TotalMemory != float64(4*MiB) {
		t.Fatalf("cached TotalMemory = %v, want %d", second.TotalMemory, 4*MiB)
	}
}

func TestGetCalculatedValuesConcurrentReset(t *testing.T) {
	fraction := 0.5
	gc := &GpuContainer{GpuFraction: &fraction}
	devInfo := &DeviceInfo{TotalMemory: 8 * MiB}

	var wg sync.WaitGroup
	for i := 0; i < 20; i++ {
		wg.Add(2)
		go func() {
			defer wg.Done()
			if got := gc.GetCalculatedValues(devInfo); got == nil {
				t.Error("GetCalculatedValues() = nil, want value")
			}
		}()
		go func() {
			defer wg.Done()
			gc.ResetCalculatedValues()
		}()
	}
	wg.Wait()
}

func equalStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}
