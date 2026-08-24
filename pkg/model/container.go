// SPDX-License-Identifier: Apache-2.0

// Package model implements the domain model of this exporter
package model

import (
	"fmt"
	"log"
	"sync"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/value"
	"k8s.io/apimachinery/pkg/types"
)

type GpuAllocationType int

const (
	K8sResource GpuAllocationType = iota
	KaiScheduler
)

func (gat GpuAllocationType) String() (s string) {
	switch gat {
	case K8sResource:
		s = "K8sResource"
	case KaiScheduler:
		s = "KaiScheduler"
	}
	return
}

type GpuContainerKey struct {
	ContainerId string
	PodUid      types.UID
}

func (k *GpuContainerKey) String() (s string) {
	if k != nil {
		s = string(k.PodUid) + "/" + k.ContainerId
	}
	return
}

type CalculatedValues struct {
	Requests    float64
	Limits      float64
	TotalMemory float64
}

type DeviceModelFingerprint struct {
	ModelName       string
	Fingerprint     uint64
	isSet           bool
	PerDeviceLabels []string
}

func (dmf *DeviceModelFingerprint) ConditionallySet(uuid, modelName string, containerLabels []string) {
	if dmf == nil {
		return
	}
	if dmf.ModelName == value.Empty {
		dmf.ModelName = modelName
	}
	if !dmf.isSet && len(containerLabels) > 0 {
		dmf.PerDeviceLabels = append([]string(nil), containerLabels...)
		dmf.PerDeviceLabels = append(dmf.PerDeviceLabels, uuid, dmf.ModelName)
		dmf.Fingerprint = value.Fingerprint(dmf.PerDeviceLabels)
		dmf.isSet = true
	}
}

type LabelFingerprint struct {
	Labels      []string
	Fingerprint uint64
}

type GpuContainer struct {
	*GpuContainerKey
	NodeName           string
	Namespace          string
	PodName            string
	ContainerName      string
	GpuAllocationType  GpuAllocationType
	GpuUuid            string
	GpuRequest         *int64
	GpuLimit           *int64
	GpuFraction        *float64
	GpuMemory          *uint64
	GPUNumDevices      *uint64
	FinishedAt         time.Time
	calculatedValuesMu sync.Mutex
	calculatedValues   *CalculatedValues
	DeviceLabels       map[string]*DeviceModelFingerprint
}

func (gc *GpuContainer) GetCalculatedValues(devInfo *DeviceInfo) *CalculatedValues {
	if gc == nil {
		return nil
	}
	gc.calculatedValuesMu.Lock()
	defer gc.calculatedValuesMu.Unlock()
	if gc.calculatedValues == nil {
		gcc := &gpuContainerCalculator{
			gc:      gc,
			devInfo: devInfo,
		}
		gcc.calculate()
	}
	return gc.calculatedValues
}

func (gc *GpuContainer) ResetCalculatedValues() {
	if gc == nil {
		return
	}
	gc.calculatedValuesMu.Lock()
	defer gc.calculatedValuesMu.Unlock()
	gc.calculatedValues = nil
}

func (gc *GpuContainer) ResetDeviceLabelCache() {
	if gc == nil {
		return
	}
	for _, device := range gc.DeviceLabels {
		if device == nil {
			continue
		}
		device.Fingerprint = 0
		device.isSet = false
		device.PerDeviceLabels = nil
	}
}

func (gc *GpuContainer) GetContainerLabels() []string {
	return []string{gc.NodeName, gc.Namespace, gc.PodName, gc.ContainerName, gc.ContainerId, gc.GpuAllocationType.String()}
}

func (gc *GpuContainer) GetPerDeviceLabels(deviceKey string) (labels []string) {
	labels, _ = gc.GetPerDeviceLabelsFingerprint(deviceKey)
	return
}

func (gc *GpuContainer) GetPerDeviceLabelsFingerprint(deviceKey string) (labels []string, fingerprint uint64) {
	if modelFingerprint := gc.DeviceLabels[deviceKey]; modelFingerprint != nil {
		modelFingerprint.ConditionallySet(deviceKey, modelFingerprint.ModelName, gc.GetContainerLabels())
		labels = modelFingerprint.PerDeviceLabels
		fingerprint = modelFingerprint.Fingerprint
		return
	}
	labels = gc.GetContainerLabels()
	return
}

func (gc *GpuContainer) GetAllPerDeviceLabels() (allLabels [][]string) {
	for deviceKey := range gc.DeviceLabels {
		allLabels = append(allLabels, gc.GetPerDeviceLabels(deviceKey))
	}
	return
}

func (gc *GpuContainer) GetAllPerDeviceLabelFingerprints() (all []LabelFingerprint) {
	for deviceKey := range gc.DeviceLabels {
		labels, fingerprint := gc.GetPerDeviceLabelsFingerprint(deviceKey)
		all = append(all, LabelFingerprint{
			Labels:      labels,
			Fingerprint: fingerprint,
		})
	}
	return
}

type gpuContainerCalculator struct {
	gc      *GpuContainer
	devInfo *DeviceInfo
}

func (gcc *gpuContainerCalculator) calculate() {
	if requests, limits, totalMemory, err := gcc.gc.calculate(gcc.devInfo); err == nil {
		gcc.gc.calculatedValues = &CalculatedValues{
			Requests:    requests,
			Limits:      limits,
			TotalMemory: totalMemory,
		}
	} else {
		log.Printf("Error calculating GPU resources for container %s: %v", gcc.gc.String(), err)
	}

}
func (gc *GpuContainer) calculate(devInfo *DeviceInfo) (requests, limits, totalMemory float64, err error) {
	if gc == nil || devInfo == nil {
		err = fmt.Errorf("gpu container or device info is nil")
		return
	}
	if devInfo.TotalMemory == 0 {
		err = fmt.Errorf("device total memory is zero")
		return
	}
	totalMemory = float64(devInfo.TotalMemory)
	if gc.GpuRequest != nil && gc.GpuLimit != nil {
		requests = float64(*gc.GpuRequest)
		limits = float64(*gc.GpuLimit)
		return
	}
	var reqLim float64
	if gc.GpuFraction != nil {
		reqLim = *gc.GpuFraction
		totalMemory *= reqLim
	} else if gc.GpuMemory != nil {
		totalMemory = float64(*gc.GpuMemory * MiB)
		reqLim = totalMemory / float64(devInfo.TotalMemory)
	} else {
		err = fmt.Errorf("no KAI annotations to determine fractional GPU request/limit")
		return
	}
	if gc.GPUNumDevices != nil {
		reqLim *= float64(*gc.GPUNumDevices)
	}
	requests, limits = reqLim, reqLim
	return
}

type GpuContainerMetrics struct {
	MemoryBytes              uint64
	MemoryTotalBytes         uint64
	ProtectedMemoryBytes     uint64
	SmUtil                   float64
	EncUtil                  float64
	DecUtil                  float64
	MemUtil                  float64
	OfaUtil                  float64
	JpgUtil                  float64
	AccountingGpuUtil        uint32
	AccountingMemUtil        uint32
	AccountingMaxMemoryBytes uint64
	AccountingTimeUs         uint64
	AccountingStartTimeUs    uint64
	AccountingIsRunning      uint32
}
