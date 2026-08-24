// SPDX-License-Identifier: Apache-2.0

package nvml

import (
	"reflect"
	"sync"
	"testing"
	"unsafe"

	nvmlapi "github.com/NVIDIA/go-nvml/pkg/nvml"
	nvmlmock "github.com/NVIDIA/go-nvml/pkg/nvml/mock"
	"github.com/densify-dev/gpu-process-exporter/pkg/kube"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	gpuprom "github.com/densify-dev/gpu-process-exporter/pkg/prometheus"
	clientprom "github.com/prometheus/client_golang/prometheus"
	dto "github.com/prometheus/client_model/go"
	"k8s.io/apimachinery/pkg/types"
)

const (
	testNodeNameNVML      = "node-a"
	testNamespaceNVML     = "ns-a"
	testPodNameNVML       = "pod-a"
	testContainerNameNVML = "ctr-a"
	testContainerIDNVML   = "container-id-a"
	testHelpNVML          = "test metric"
)

func TestGroupProcessUtilizationInfo(t *testing.T) {
	samples := []nvmlapi.ProcessUtilizationInfo_v1{
		{Pid: 20, TimeStamp: 200, SmUtil: 1},
		{Pid: 20, TimeStamp: 240, SmUtil: 2},
		{Pid: 21, TimeStamp: 230, SmUtil: 3},
		{Pid: 22, TimeStamp: 180, SmUtil: 4},
		{Pid: 20, TimeStamp: 240, SmUtil: 5},
	}
	testGroupProcessUtilization(t, samples)
}

func TestGroupProcessUtilizationSamples(t *testing.T) {
	samples := []nvmlapi.ProcessUtilizationSample{
		{Pid: 20, TimeStamp: 200, SmUtil: 1},
		{Pid: 20, TimeStamp: 240, SmUtil: 2},
		{Pid: 21, TimeStamp: 230, SmUtil: 3},
		{Pid: 22, TimeStamp: 180, SmUtil: 4},
		{Pid: 20, TimeStamp: 240, SmUtil: 5},
	}
	testGroupProcessUtilization(t, samples)
}

func testGroupProcessUtilization[U Utilization](t *testing.T, us []U) {
	groups, maxSeenTs := groupUtilizations(us, 200)
	if maxSeenTs != 240 {
		t.Fatalf("maxSeenTs = %d, want 240", maxSeenTs)
	}
	if len(groups) != 2 {
		t.Fatalf("len(groups) = %d, want 2", len(groups))
	}
	if groups[0].timestamp != 230 || groups[1].timestamp != 240 {
		t.Fatalf("timestamps = %d, %d; want 230, 240", groups[0].timestamp, groups[1].timestamp)
	}
	if got := getSmUtil(groups[0].utilizations[21]); got != 3 {
		t.Fatalf("pid 21 SmUtil = %d, want 3", got)
	}
	if got := getSmUtil(groups[1].utilizations[20]); got != 5 {
		t.Fatalf("pid 20 duplicate timestamp SmUtil = %d, want 5", got)
	}
	if _, ok := groups[0].utilizations[22]; ok {
		t.Fatalf("pid 22 should have been filtered as stale")
	}
}

func getSmUtil[U Utilization](u U) (smUtil uint32) {
	switch ut := any(u).(type) {
	case nvmlapi.ProcessUtilizationSample:
		smUtil = ut.SmUtil
	case nvmlapi.ProcessUtilizationInfo_v1:
		smUtil = ut.SmUtil
	}
	return
}

func TestSampleWeightSecondsUsesDefaultForFirstSample(t *testing.T) {
	mc := &MetricsCollector{defaultUtilSampleInterval: 2.5}

	if got := mc.sampleWeightSeconds(0, 40); got != 2.5 {
		t.Fatalf("sampleWeightSeconds first sample = %v, want 2.5", got)
	}
	if got := mc.sampleWeightSeconds(1_000_000, 3_500_000); got != 2.5 {
		t.Fatalf("sampleWeightSeconds delta = %v, want 2.5", got)
	}
}

func TestEnrichGpuMetricsSetsProtectedMemoryWithoutAccounting(t *testing.T) {
	gpuMetrics := &model.GpuContainerMetrics{}
	processDetailMap := map[uint32]nvmlapi.ProcessDetail_v1{
		7: {Pid: 7, UsedGpuCcProtectedMemory: 64},
	}

	(&MetricsCollector{}).enrichGpuMetrics(
		gpuMetrics,
		&model.DeviceInfo{AccountingEnabled: false},
		&nvmlmock.Device{},
		7,
		processDetailMap,
		make(map[uint32]nvmlapi.AccountingStats),
		make(map[uint32]bool),
	)

	if gpuMetrics.ProtectedMemoryBytes != 64 {
		t.Fatalf("ProtectedMemoryBytes = %d, want 64", gpuMetrics.ProtectedMemoryBytes)
	}
	if gpuMetrics.AccountingGpuUtil != 0 {
		t.Fatalf("AccountingGpuUtil = %d, want 0", gpuMetrics.AccountingGpuUtil)
	}
}

func TestEnrichGpuMetricsCachesAccountingStats(t *testing.T) {
	device := &nvmlmock.Device{
		GetAccountingStatsFunc: func(pid uint32) (nvmlapi.AccountingStats, nvmlapi.Return) {
			return nvmlapi.AccountingStats{
				GpuUtilization:    10,
				MemoryUtilization: 20,
				MaxMemoryUsage:    30,
				Time:              40,
				StartTime:         50,
				IsRunning:         1,
			}, nvmlapi.SUCCESS
		},
	}
	gpuMetrics := &model.GpuContainerMetrics{}
	accountingStatsMap := make(map[uint32]nvmlapi.AccountingStats)
	accountingStatsKnown := make(map[uint32]bool)
	mc := &MetricsCollector{}

	mc.enrichGpuMetrics(
		gpuMetrics,
		&model.DeviceInfo{AccountingEnabled: true},
		device,
		11,
		nil,
		accountingStatsMap,
		accountingStatsKnown,
	)
	mc.enrichGpuMetrics(
		gpuMetrics,
		&model.DeviceInfo{AccountingEnabled: true},
		device,
		11,
		nil,
		accountingStatsMap,
		accountingStatsKnown,
	)

	if got := len(device.GetAccountingStatsCalls()); got != 1 {
		t.Fatalf("GetAccountingStats call count = %d, want 1", got)
	}
	if gpuMetrics.AccountingGpuUtil != 10 || gpuMetrics.AccountingMemUtil != 20 {
		t.Fatalf("unexpected accounting util values: %+v", gpuMetrics)
	}
	if gpuMetrics.AccountingMaxMemoryBytes != 30 ||
		gpuMetrics.AccountingTimeUs != 40 ||
		gpuMetrics.AccountingStartTimeUs != 50 ||
		gpuMetrics.AccountingIsRunning != 1 {
		t.Fatalf("unexpected accounting stats: %+v", gpuMetrics)
	}
}

func TestGetProcessDetailMap(t *testing.T) {
	details := []nvmlapi.ProcessDetail_v1{
		{Pid: 21, UsedGpuCcProtectedMemory: 128},
		{Pid: 22, UsedGpuCcProtectedMemory: 256},
	}
	device := &nvmlmock.Device{
		GetRunningProcessDetailListFunc: func() (nvmlapi.ProcessDetailList, nvmlapi.Return) {
			return nvmlapi.ProcessDetailList{
				NumProcArrayEntries: uint32(len(details)),
				ProcArray:           &details[0],
			}, nvmlapi.SUCCESS
		},
	}

	got := (&MetricsCollector{}).getProcessDetailMap(device)

	if len(got) != 2 {
		t.Fatalf("len(got) = %d, want 2", len(got))
	}
	if got[21].UsedGpuCcProtectedMemory != 128 || got[22].UsedGpuCcProtectedMemory != 256 {
		t.Fatalf("unexpected process detail map: %+v", got)
	}
}

func TestGetRunningProcessMemoryIncludesGraphicsOnlyProcesses(t *testing.T) {
	device := &nvmlmock.Device{
		GetComputeRunningProcessesFunc: func() ([]nvmlapi.ProcessInfo, nvmlapi.Return) {
			return []nvmlapi.ProcessInfo{
				{Pid: 31, UsedGpuMemory: 128},
			}, nvmlapi.SUCCESS
		},
		GetGraphicsRunningProcessesFunc: func() ([]nvmlapi.ProcessInfo, nvmlapi.Return) {
			return []nvmlapi.ProcessInfo{
				{Pid: 32, UsedGpuMemory: 256},
			}, nvmlapi.SUCCESS
		},
	}

	got := getRunningProcessMemory(device)

	if len(got) != 2 {
		t.Fatalf("len(got) = %d, want 2: %+v", len(got), got)
	}
	if got[31] != 128 {
		t.Fatalf("compute process memory = %d, want 128", got[31])
	}
	if got[32] != 256 {
		t.Fatalf("graphics process memory = %d, want 256", got[32])
	}
}

func TestGetRunningProcessMemoryDeduplicatesComputeAndGraphicsProcesses(t *testing.T) {
	device := &nvmlmock.Device{
		GetComputeRunningProcessesFunc: func() ([]nvmlapi.ProcessInfo, nvmlapi.Return) {
			return []nvmlapi.ProcessInfo{
				{Pid: 41, UsedGpuMemory: 128},
			}, nvmlapi.SUCCESS
		},
		GetGraphicsRunningProcessesFunc: func() ([]nvmlapi.ProcessInfo, nvmlapi.Return) {
			return []nvmlapi.ProcessInfo{
				{Pid: 41, UsedGpuMemory: 256},
			}, nvmlapi.SUCCESS
		},
	}

	got := getRunningProcessMemory(device)

	if len(got) != 1 {
		t.Fatalf("len(got) = %d, want 1: %+v", len(got), got)
	}
	if got[41] != 256 {
		t.Fatalf("deduplicated process memory = %d, want 256", got[41])
	}
}

func TestReportMetricsSetsGaugesAndCounters(t *testing.T) {
	const (
		podUID      = types.UID("pod-uid-a")
		containerID = testContainerIDNVML
		uuid        = "gpu-uuid-a"
	)
	request := int64(1)
	limit := int64(2)
	gc := &model.GpuContainer{
		GpuContainerKey: &model.GpuContainerKey{
			PodUid:      podUID,
			ContainerId: containerID,
		},
		NodeName:      testNodeNameNVML,
		Namespace:     testNamespaceNVML,
		PodName:       testPodNameNVML,
		ContainerName: testContainerNameNVML,
		GpuRequest:    &request,
		GpuLimit:      &limit,
		DeviceLabels:  map[string]*model.DeviceModelFingerprint{},
	}
	store := &kube.Store{}
	setUnexportedField(store, "gpuContainers", map[types.UID]map[string]*model.GpuContainer{
		podUID: {
			containerID: gc,
		},
	})
	setUnexportedField(store, "gpuContainersMu", sync.RWMutex{})

	gmp := newTestGpuMetricsProvider()
	mc := &MetricsCollector{
		store: store,
		gmp:   gmp,
		devices: map[string]*model.DeviceInfo{
			uuid: {
				Uuid:              uuid,
				ModelName:         "model-a",
				TotalMemory:       2 * model.MiB,
				AccountingEnabled: true,
			},
		},
	}
	currentMetrics := map[model.GpuContainerKey]map[string]*model.GpuContainerMetrics{
		*gc.GpuContainerKey: {
			uuid: {
				MemoryBytes:              512,
				ProtectedMemoryBytes:     8,
				SmUtil:                   4,
				MemUtil:                  5,
				EncUtil:                  6,
				DecUtil:                  7,
				OfaUtil:                  8,
				JpgUtil:                  9,
				AccountingGpuUtil:        10,
				AccountingMemUtil:        11,
				AccountingMaxMemoryBytes: 12,
				AccountingTimeUs:         13,
				AccountingStartTimeUs:    14,
				AccountingIsRunning:      1,
			},
		},
	}
	containerLabels := []string{
		testNodeNameNVML,
		testNamespaceNVML,
		testPodNameNVML,
		testContainerNameNVML,
		testContainerIDNVML,
		model.K8sResource.String(),
	}
	perDeviceLabels := []string{
		testNodeNameNVML,
		testNamespaceNVML,
		testPodNameNVML,
		testContainerNameNVML,
		testContainerIDNVML,
		model.K8sResource.String(),
		uuid,
		"model-a",
	}

	mc.reportMetrics(currentMetrics)

	if got := gaugeValue(t, gmp.Requests.WithLabelValues(containerLabels...)); got != 1 {
		t.Fatalf("Requests = %v, want 1", got)
	}
	if got := gaugeValue(t, gmp.Limits.WithLabelValues(containerLabels...)); got != 2 {
		t.Fatalf("Limits = %v, want 2", got)
	}
	if got := gaugeValue(t, gmp.MemoryTotalBytes.WithLabelValues(perDeviceLabels...)); got != 2*model.MiB {
		t.Fatalf("MemoryTotalBytes = %v, want %d", got, 2*model.MiB)
	}
	if got := gaugeValue(t, gmp.MemoryBytes.WithLabelValues(perDeviceLabels...)); got != 512 {
		t.Fatalf("MemoryBytes = %v, want 512", got)
	}
	if got := gaugeValue(t, gmp.ProtectedMemoryBytes.WithLabelValues(perDeviceLabels...)); got != 8 {
		t.Fatalf("ProtectedMemoryBytes = %v, want 8", got)
	}
	if got := counterValue(t, gmp.SmUtil.WithLabelValues(perDeviceLabels...)); got != 4 {
		t.Fatalf("SmUtil counter = %v, want 4", got)
	}
	if got := counterValue(t, gmp.MemoryUtil.WithLabelValues(perDeviceLabels...)); got != 5 {
		t.Fatalf("MemoryUtil counter = %v, want 5", got)
	}
	if got := counterValue(t, gmp.EncUtil.WithLabelValues(perDeviceLabels...)); got != 6 {
		t.Fatalf("EncUtil counter = %v, want 6", got)
	}
	if got := counterValue(t, gmp.DecUtil.WithLabelValues(perDeviceLabels...)); got != 7 {
		t.Fatalf("DecUtil counter = %v, want 7", got)
	}
	if got := counterValue(t, gmp.OfaUtil.WithLabelValues(perDeviceLabels...)); got != 8 {
		t.Fatalf("OfaUtil counter = %v, want 8", got)
	}
	if got := counterValue(t, gmp.JpgUtil.WithLabelValues(perDeviceLabels...)); got != 9 {
		t.Fatalf("JpgUtil counter = %v, want 9", got)
	}
	if got := gaugeValue(t, gmp.AccountingGpuUtil.WithLabelValues(perDeviceLabels...)); got != 10 {
		t.Fatalf("AccountingGpuUtil gauge = %v, want 10", got)
	}
	if got := gaugeValue(t, gmp.AccountingMemUtil.WithLabelValues(perDeviceLabels...)); got != 11 {
		t.Fatalf("AccountingMemUtil gauge = %v, want 11", got)
	}
	if got := gaugeValue(t, gmp.AccountingMaxMemoryBytes.WithLabelValues(perDeviceLabels...)); got != 12 {
		t.Fatalf("AccountingMaxMemoryBytes = %v, want 12", got)
	}
	if got := gaugeValue(t, gmp.AccountingTimeUs.WithLabelValues(perDeviceLabels...)); got != 13 {
		t.Fatalf("AccountingTimeUs = %v, want 13", got)
	}
	if got := gaugeValue(t, gmp.AccountingStartTimeUs.WithLabelValues(perDeviceLabels...)); got != 14 {
		t.Fatalf("AccountingStartTimeUs = %v, want 14", got)
	}
	if got := gaugeValue(t, gmp.AccountingIsRunning.WithLabelValues(perDeviceLabels...)); got != 1 {
		t.Fatalf("AccountingIsRunning = %v, want 1", got)
	}
	wantFootprint := 100.0 * 512 / float64(2*model.MiB)
	if got := gaugeValue(t, gmp.MemoryFootprint.WithLabelValues(perDeviceLabels...)); got != wantFootprint {
		t.Fatalf("MemoryFootprint gauge = %v, want %v", got, wantFootprint)
	}
}

func newTestGpuMetricsProvider() *gpuprom.GpuMetricsProvider {
	containerLabels := []string{
		gpuprom.LabelNode,
		gpuprom.LabelNamespace,
		gpuprom.LabelPod,
		gpuprom.LabelContainer,
		gpuprom.LabelContainerId,
		gpuprom.LabelGpuAllocationType,
	}
	perDeviceLabels := append(append([]string{}, containerLabels...), gpuprom.LabelGpuUuid, gpuprom.LabelGpuModel)
	protectedMemoryBytes := clientprom.NewGaugeVec(
		clientprom.GaugeOpts{Name: "test_nvml_protected_memory_bytes", Help: testHelpNVML},
		perDeviceLabels,
	)

	gmp := &gpuprom.GpuMetricsProvider{
		Requests:                 newTestGaugeVec("test_nvml_requests", containerLabels),
		Limits:                   newTestGaugeVec("test_nvml_limits", containerLabels),
		MemoryBytes:              newTestGaugeVec("test_nvml_memory_bytes", perDeviceLabels),
		MemoryTotalBytes:         newTestGaugeVec("test_nvml_memory_total_bytes", perDeviceLabels),
		MemoryFootprint:          newTestGaugeVec("test_nvml_memory_footprint", perDeviceLabels),
		SmUtil:                   newTestCounterVec("test_nvml_sm_util", perDeviceLabels),
		MemoryUtil:               newTestCounterVec("test_nvml_memory_util", perDeviceLabels),
		EncUtil:                  newTestCounterVec("test_nvml_enc_util", perDeviceLabels),
		DecUtil:                  newTestCounterVec("test_nvml_dec_util", perDeviceLabels),
		OfaUtil:                  newTestCounterVec("test_nvml_ofa_util", perDeviceLabels),
		JpgUtil:                  newTestCounterVec("test_nvml_jpg_util", perDeviceLabels),
		ProtectedMemoryBytes:     protectedMemoryBytes,
		AccountingGpuUtil:        newTestGaugeVec("test_nvml_accounting_gpu_util", perDeviceLabels),
		AccountingMemUtil:        newTestGaugeVec("test_nvml_accounting_mem_util", perDeviceLabels),
		AccountingMaxMemoryBytes: newTestGaugeVec("test_nvml_accounting_max_memory_bytes", perDeviceLabels),
		AccountingTimeUs:         newTestGaugeVec("test_nvml_accounting_time_us", perDeviceLabels),
		AccountingStartTimeUs:    newTestGaugeVec("test_nvml_accounting_start_time_us", perDeviceLabels),
		AccountingIsRunning:      newTestGaugeVec("test_nvml_accounting_is_running", perDeviceLabels),
	}
	setUnexportedField(gmp, "fingerprintsByGauge", map[*clientprom.GaugeVec]map[uint64]bool{
		protectedMemoryBytes: make(map[uint64]bool),
	})
	return gmp
}

func setUnexportedField(target any, name string, value any) {
	field := reflect.ValueOf(target).Elem().FieldByName(name)
	reflect.NewAt(field.Type(), unsafe.Pointer(field.UnsafeAddr())).Elem().Set(reflect.ValueOf(value))
}

func gaugeValue(t *testing.T, metric clientprom.Metric) float64 {
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

func counterValue(t *testing.T, metric clientprom.Metric) float64 {
	t.Helper()

	var m dto.Metric
	if err := metric.Write(&m); err != nil {
		t.Fatalf("write metric: %v", err)
	}
	if m.Counter == nil {
		t.Fatalf("metric is not a counter")
	}
	return m.GetCounter().GetValue()
}

func newTestGaugeVec(name string, labels []string) *clientprom.GaugeVec {
	return clientprom.NewGaugeVec(clientprom.GaugeOpts{Name: name, Help: testHelpNVML}, labels)
}

func newTestCounterVec(name string, labels []string) *clientprom.CounterVec {
	return clientprom.NewCounterVec(clientprom.CounterOpts{Name: name, Help: testHelpNVML}, labels)
}
