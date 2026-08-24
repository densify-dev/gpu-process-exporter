// SPDX-License-Identifier: Apache-2.0

// Package nvml implements the GPU metrics collection using NVML library
package nvml

import (
	"context"
	"fmt"
	"log"
	"sort"
	"time"
	"unsafe"

	"github.com/NVIDIA/go-nvml/pkg/nvml"
	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/kube"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	"github.com/densify-dev/gpu-process-exporter/pkg/prometheus"
	"github.com/densify-dev/gpu-process-exporter/pkg/value"
)

type MetricsCollector struct {
	store                            *kube.Store
	gmp                              *prometheus.GpuMetricsProvider
	cfg                              *config.Config
	devices                          map[string]*model.DeviceInfo
	utilizationLastSeenByGpu         map[string]uint64
	extendedUtilizationLastSeenByGpu map[string]uint64
	defaultUtilSampleInterval        float64
}

func NewMetricsCollector(
	store *kube.Store,
	cfg *config.Config,
	gmp *prometheus.GpuMetricsProvider,
) *MetricsCollector {
	pollIntervalSeconds := cfg.DriverPollInterval.Seconds()
	return &MetricsCollector{store: store,
		gmp:                              gmp,
		cfg:                              cfg,
		devices:                          make(map[string]*model.DeviceInfo),
		utilizationLastSeenByGpu:         make(map[string]uint64),
		extendedUtilizationLastSeenByGpu: make(map[string]uint64),
		defaultUtilSampleInterval:        pollIntervalSeconds,
	}
}

func (mc *MetricsCollector) Run(ctx context.Context) (err error) {
	var ret nvml.Return
	if ret = nvml.Init(); ret != nvml.SUCCESS {
		err = fmt.Errorf("NVML init failed: %s", nvml.ErrorString(ret))
		return
	}
	defer func() {
		if ret := nvml.Shutdown(); ret != nvml.SUCCESS {
			shutdownErr := fmt.Errorf("NVML shutdown failed: %s", nvml.ErrorString(ret))
			if err == nil {
				err = shutdownErr
			} else {
				log.Printf("nvml: additionally, shutdown failed: %v", shutdownErr)
			}
		}
	}()
	ticker := time.NewTicker(mc.cfg.DriverPollInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			mc.collect()
		}
	}
}

func (mc *MetricsCollector) collect() {
	currentMetrics := make(map[model.GpuContainerKey]map[string]*model.GpuContainerMetrics)

	deviceCount, ret := nvml.DeviceGetCount()
	if ret != nvml.SUCCESS {
		log.Printf("nvml: get device count failed: %s", nvml.ErrorString(ret))
		return
	}

	for i := 0; i < deviceCount; i++ {
		device, ret := nvml.DeviceGetHandleByIndex(i)
		if ret != nvml.SUCCESS {
			log.Printf("nvml: get device handle for index %d failed: %s", i, nvml.ErrorString(ret))
			continue
		}

		uuid, ret := device.GetUUID()
		if ret != nvml.SUCCESS || uuid == value.Empty {
			log.Printf("nvml: get uuid for device index %d failed: %s", i, nvml.ErrorString(ret))
			continue
		}

		info, ok := mc.devices[uuid]
		if !ok {
			modelName, ret := device.GetName()
			if ret != nvml.SUCCESS {
				log.Printf("nvml: get model name for device %s failed: %s", uuid, nvml.ErrorString(ret))
				continue
			}

			var totalMemory uint64
			if memoryInfo2, ret := device.GetMemoryInfo_v2(); ret == nvml.SUCCESS && memoryInfo2.Total > 0 {
				totalMemory = memoryInfo2.Total
			} else if memoryInfo, ret := device.GetMemoryInfo(); ret == nvml.SUCCESS && memoryInfo.Total > 0 {
				totalMemory = memoryInfo.Total
			}
			if totalMemory == 0 {
				log.Printf("nvml: get total memory for device %s failed", uuid)
				continue
			}

			var accountingEnabled bool
			if mode, ret := device.GetAccountingMode(); ret == nvml.SUCCESS {
				accountingEnabled = mode == nvml.FEATURE_ENABLED
			}

			info = &model.DeviceInfo{
				Uuid:              uuid,
				ModelName:         modelName,
				TotalMemory:       totalMemory,
				AccountingEnabled: accountingEnabled,
			}
			mc.devices[uuid] = info
		}

		processDetailMap := mc.getProcessDetailMap(device)
		accountingStatsMap := make(map[uint32]nvml.AccountingStats)
		accountingStatsKnown := make(map[uint32]bool)
		enrichedPids := make(map[uint32]bool)

		for pid, usedGpuMemory := range getRunningProcessMemory(device) {
			gpuMetrics, ok := mc.getOrCreateGpuMetrics(currentMetrics, uuid, pid)
			if !ok {
				continue
			}
			gpuMetrics.MemoryBytes = usedGpuMemory
			gpuMetrics.MemoryTotalBytes = info.TotalMemory
			if !enrichedPids[pid] {
				mc.enrichGpuMetrics(
					gpuMetrics,
					info,
					device,
					pid,
					processDetailMap,
					accountingStatsMap,
					accountingStatsKnown,
				)
				enrichedPids[pid] = true
			}
		}
		mc.collectUtilization(
			uuid,
			device,
			info,
			currentMetrics,
			processDetailMap,
			accountingStatsMap,
			enrichedPids,
			accountingStatsKnown,
		)
	}

	mc.reportMetrics(currentMetrics)
}

func (mc *MetricsCollector) getOrCreateGpuMetrics(
	currentMetrics map[model.GpuContainerKey]map[string]*model.GpuContainerMetrics,
	uuid string,
	pid uint32,
) (*model.GpuContainerMetrics, bool) {
	gck, err := mc.store.GetMapper().GetContainerKey(model.Pid(pid))
	if err != nil {
		log.Printf("nvml: get container key for pid %d: %v", pid, err)
		return nil, false
	}
	if gck == nil {
		return nil, false
	}

	perContainerMetrics, ok := currentMetrics[*gck]
	if !ok {
		perContainerMetrics = make(map[string]*model.GpuContainerMetrics)
		currentMetrics[*gck] = perContainerMetrics
	}

	gpuMetrics, ok := perContainerMetrics[uuid]
	if !ok {
		gpuMetrics = &model.GpuContainerMetrics{}
		perContainerMetrics[uuid] = gpuMetrics
	}

	return gpuMetrics, true
}

func (mc *MetricsCollector) collectUtilization(
	uuid string,
	device nvml.Device,
	info *model.DeviceInfo,
	currentMetrics map[model.GpuContainerKey]map[string]*model.GpuContainerMetrics,
	processDetailMap map[uint32]nvml.ProcessDetail_v1,
	accountingStatsMap map[uint32]nvml.AccountingStats,
	enrichedPids map[uint32]bool,
	accountingStatsKnown map[uint32]bool,
) {
	lastSeenUtilTs := mc.utilizationLastSeenByGpu[uuid]
	utilSamples, ret := device.GetProcessUtilization(lastSeenUtilTs)
	if ret == nvml.SUCCESS && len(utilSamples) > 0 {
		sampleGroups, maxSeenTs := groupUtilizations(utilSamples, lastSeenUtilTs)
		previousTs := lastSeenUtilTs
		for _, group := range sampleGroups {
			weight := mc.sampleWeightSeconds(previousTs, group.timestamp)
			for _, sample := range group.utilizations {
				gpuMetrics, ok := mc.getOrCreateGpuMetrics(currentMetrics, uuid, sample.Pid)
				if !ok {
					continue
				}
				gpuMetrics.SmUtil += float64(sample.SmUtil) * weight
				gpuMetrics.MemUtil += float64(sample.MemUtil) * weight
				gpuMetrics.EncUtil += float64(sample.EncUtil) * weight
				gpuMetrics.DecUtil += float64(sample.DecUtil) * weight
				if !enrichedPids[sample.Pid] {
					mc.enrichGpuMetrics(
						gpuMetrics,
						info,
						device,
						sample.Pid,
						processDetailMap,
						accountingStatsMap,
						accountingStatsKnown,
					)
					enrichedPids[sample.Pid] = true
				}
			}
			previousTs = group.timestamp
		}
		if maxSeenTs > lastSeenUtilTs {
			mc.utilizationLastSeenByGpu[uuid] = maxSeenTs
		}
	}

	lastSeenExtUtilTs := mc.extendedUtilizationLastSeenByGpu[uuid]
	utilizationInfos, ret := device.GetProcessesUtilizationInfo()
	if ret == nvml.SUCCESS && utilizationInfos.ProcessSamplesCount > 0 && utilizationInfos.ProcUtilArray != nil {
		utilInfos := unsafe.Slice(utilizationInfos.ProcUtilArray, int(utilizationInfos.ProcessSamplesCount))
		infoGroups, maxSeenTs := groupUtilizations(utilInfos, lastSeenExtUtilTs)
		previousTs := lastSeenExtUtilTs
		for _, group := range infoGroups {
			weight := mc.sampleWeightSeconds(previousTs, group.timestamp)
			for _, utilInfo := range group.utilizations {
				gpuMetrics, ok := mc.getOrCreateGpuMetrics(currentMetrics, uuid, utilInfo.Pid)
				if !ok {
					continue
				}
				gpuMetrics.OfaUtil += float64(utilInfo.OfaUtil) * weight
				gpuMetrics.JpgUtil += float64(utilInfo.JpgUtil) * weight
				if !enrichedPids[utilInfo.Pid] {
					mc.enrichGpuMetrics(
						gpuMetrics,
						info,
						device,
						utilInfo.Pid,
						processDetailMap,
						accountingStatsMap,
						accountingStatsKnown,
					)
					enrichedPids[utilInfo.Pid] = true
				}
			}
			previousTs = group.timestamp
		}
		if maxSeenTs > lastSeenExtUtilTs {
			mc.extendedUtilizationLastSeenByGpu[uuid] = maxSeenTs
		}
	}
}

func getRunningProcessMemory(device nvml.Device) map[uint32]uint64 {
	processMemory := make(map[uint32]uint64)
	if processes, ret := device.GetComputeRunningProcesses(); ret == nvml.SUCCESS {
		addRunningProcessMemory(processMemory, processes)
	}
	if processes, ret := device.GetGraphicsRunningProcesses(); ret == nvml.SUCCESS {
		addRunningProcessMemory(processMemory, processes)
	}
	return processMemory
}

func addRunningProcessMemory(processMemory map[uint32]uint64, processes []nvml.ProcessInfo) {
	for _, proc := range processes {
		if proc.UsedGpuMemory > processMemory[proc.Pid] {
			processMemory[proc.Pid] = proc.UsedGpuMemory
		}
	}
}

type Utilization interface {
	nvml.ProcessUtilizationSample | nvml.ProcessUtilizationInfo_v1
}

type processUtilizationGroup[U Utilization] struct {
	timestamp    uint64
	utilizations map[uint32]U
}

func getUtilizationTimestamp[U Utilization](u U) (ts uint64) {
	switch ut := any(u).(type) {
	case nvml.ProcessUtilizationSample:
		ts = ut.TimeStamp
	case nvml.ProcessUtilizationInfo_v1:
		ts = ut.TimeStamp
	}
	return
}

func getUtilizationPid[U Utilization](u U) (pid uint32) {
	switch ut := any(u).(type) {
	case nvml.ProcessUtilizationSample:
		pid = ut.Pid
	case nvml.ProcessUtilizationInfo_v1:
		pid = ut.Pid
	}
	return
}

func groupUtilizations[U Utilization](utils []U, lastSeen uint64) ([]processUtilizationGroup[U], uint64) {
	utilizationsByTimestamp := make(map[uint64]map[uint32]U)
	maxSeenTs := lastSeen
	for _, u := range utils {
		ts := getUtilizationTimestamp(u)
		if ts <= lastSeen {
			continue
		}
		if ts > maxSeenTs {
			maxSeenTs = ts
		}
		utilizationsByPid, ok := utilizationsByTimestamp[ts]
		if !ok {
			utilizationsByPid = make(map[uint32]U)
			utilizationsByTimestamp[ts] = utilizationsByPid
		}
		utilizationsByPid[getUtilizationPid(u)] = u
	}
	return orderedUtilizationGroups(utilizationsByTimestamp), maxSeenTs
}

func orderedUtilizationGroups[U Utilization](
	utilizationsByTimestamp map[uint64]map[uint32]U,
) []processUtilizationGroup[U] {
	timestamps := sortedTimestamps(utilizationsByTimestamp)
	groups := make([]processUtilizationGroup[U], 0, len(timestamps))
	for _, ts := range timestamps {
		groups = append(groups, processUtilizationGroup[U]{
			timestamp:    ts,
			utilizations: utilizationsByTimestamp[ts],
		})
	}
	return groups
}

func sortedTimestamps[T any](samplesByTimestamp map[uint64]T) []uint64 {
	timestamps := make([]uint64, 0, len(samplesByTimestamp))
	for ts := range samplesByTimestamp {
		timestamps = append(timestamps, ts)
	}
	sort.Slice(timestamps, func(i, j int) bool {
		return timestamps[i] < timestamps[j]
	})
	return timestamps
}

const (
	microSecondsInSecond = float64(time.Second / time.Microsecond)
)

func (mc *MetricsCollector) sampleWeightSeconds(previousTs, sampleTs uint64) (wss float64) {
	if previousTs == 0 {
		if mc.defaultUtilSampleInterval > 0 {
			wss = mc.defaultUtilSampleInterval
		} else if mc.cfg != nil {
			wss = mc.cfg.DriverPollInterval.Seconds()
		}
	} else if sampleTs > previousTs {
		wss = float64(sampleTs-previousTs) / microSecondsInSecond
	}
	return
}

func (mc *MetricsCollector) enrichGpuMetrics(
	gpuMetrics *model.GpuContainerMetrics,
	info *model.DeviceInfo,
	device nvml.Device,
	pid uint32,
	processDetailMap map[uint32]nvml.ProcessDetail_v1,
	accountingStatsMap map[uint32]nvml.AccountingStats,
	accountingStatsKnown map[uint32]bool,
) {
	if gpuMetrics == nil {
		return
	}
	if detail, ok := processDetailMap[pid]; ok {
		gpuMetrics.ProtectedMemoryBytes = detail.UsedGpuCcProtectedMemory
	}
	if info == nil || !info.AccountingEnabled {
		return
	}
	stats, ok := accountingStatsMap[pid]
	if !ok && !accountingStatsKnown[pid] {
		accountingStatsKnown[pid] = true
		var ret nvml.Return
		stats, ret = device.GetAccountingStats(pid)
		if ret == nvml.SUCCESS {
			accountingStatsMap[pid] = stats
			ok = true
		}
	}
	if !ok {
		return
	}
	gpuMetrics.AccountingGpuUtil = stats.GpuUtilization
	gpuMetrics.AccountingMemUtil = stats.MemoryUtilization
	gpuMetrics.AccountingMaxMemoryBytes = stats.MaxMemoryUsage
	gpuMetrics.AccountingTimeUs = stats.Time
	gpuMetrics.AccountingStartTimeUs = stats.StartTime
	gpuMetrics.AccountingIsRunning = stats.IsRunning
}

func (mc *MetricsCollector) getProcessDetailMap(device nvml.Device) map[uint32]nvml.ProcessDetail_v1 {
	processDetailMap := make(map[uint32]nvml.ProcessDetail_v1)
	processDetails, ret := device.GetRunningProcessDetailList()
	if ret != nvml.SUCCESS || processDetails.NumProcArrayEntries == 0 || processDetails.ProcArray == nil {
		return processDetailMap
	}
	for _, detail := range unsafe.Slice(processDetails.ProcArray, int(processDetails.NumProcArrayEntries)) {
		processDetailMap[detail.Pid] = detail
	}
	return processDetailMap
}

func (mc *MetricsCollector) reportMetrics(
	currentMetrics map[model.GpuContainerKey]map[string]*model.GpuContainerMetrics,
) {
	for gck, perContainerMetrics := range currentMetrics {
		for uuid, perDeviceMetrics := range perContainerMetrics {
			devInfo := mc.devices[uuid]
			if devInfo == nil {
				continue
			}
			containerLabels, perDeviceLabels, fingerprint, calculatedValues, ok := mc.store.GetMetricLabelsAndValues(
				&gck,
				devInfo,
			)
			if !ok {
				continue
			}

			var totalMemory float64
			if calculatedValues == nil {
				totalMemory = float64(perDeviceMetrics.MemoryTotalBytes)
				if totalMemory == 0 {
					totalMemory = float64(devInfo.TotalMemory)
				}
			} else {
				prometheus.SetValue(mc.gmp.Requests, containerLabels, calculatedValues.Requests)
				prometheus.SetValue(mc.gmp.Limits, containerLabels, calculatedValues.Limits)
				totalMemory = calculatedValues.TotalMemory
			}
			prometheus.SetValue(mc.gmp.MemoryTotalBytes, perDeviceLabels, totalMemory)
			memory := float64(perDeviceMetrics.MemoryBytes)
			memoryFootprint := 100.0 * memory / totalMemory
			mc.gmp.SetMemoryValues(
				perDeviceLabels,
				fingerprint,
				memory,
				float64(perDeviceMetrics.ProtectedMemoryBytes),
				memoryFootprint,
			)
			_ = prometheus.AddValueConditional(
				mc.gmp.SmUtil,
				perDeviceLabels,
				perDeviceMetrics.SmUtil,
				prometheus.PositiveValue,
			)
			_ = prometheus.AddValueConditional(
				mc.gmp.MemoryUtil,
				perDeviceLabels,
				perDeviceMetrics.MemUtil,
				prometheus.PositiveValue,
			)
			_ = prometheus.AddValueConditional(
				mc.gmp.EncUtil,
				perDeviceLabels,
				perDeviceMetrics.EncUtil,
				prometheus.PositiveValue,
			)
			_ = prometheus.AddValueConditional(
				mc.gmp.DecUtil,
				perDeviceLabels,
				perDeviceMetrics.DecUtil,
				prometheus.PositiveValue,
			)
			_ = prometheus.AddValueConditional(
				mc.gmp.OfaUtil,
				perDeviceLabels,
				perDeviceMetrics.OfaUtil,
				prometheus.PositiveValue,
			)
			_ = prometheus.AddValueConditional(
				mc.gmp.JpgUtil,
				perDeviceLabels,
				perDeviceMetrics.JpgUtil,
				prometheus.PositiveValue,
			)
			if devInfo.AccountingEnabled {
				prometheus.SetValue(mc.gmp.AccountingGpuUtil, perDeviceLabels, float64(perDeviceMetrics.AccountingGpuUtil))
				prometheus.SetValue(mc.gmp.AccountingMemUtil, perDeviceLabels, float64(perDeviceMetrics.AccountingMemUtil))
				prometheus.SetValue(
					mc.gmp.AccountingMaxMemoryBytes,
					perDeviceLabels,
					float64(perDeviceMetrics.AccountingMaxMemoryBytes),
				)
				_ = prometheus.SetValueConditional(
					mc.gmp.AccountingTimeUs,
					perDeviceLabels,
					float64(perDeviceMetrics.AccountingTimeUs),
					prometheus.PositiveValue,
				)
				prometheus.SetValue(mc.gmp.AccountingStartTimeUs, perDeviceLabels, float64(perDeviceMetrics.AccountingStartTimeUs))
				prometheus.SetValue(mc.gmp.AccountingIsRunning, perDeviceLabels, float64(perDeviceMetrics.AccountingIsRunning))
			}
		}
	}
}
