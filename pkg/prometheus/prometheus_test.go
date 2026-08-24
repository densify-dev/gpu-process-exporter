// SPDX-License-Identifier: Apache-2.0

package prometheus

import (
	"testing"

	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	"github.com/densify-dev/gpu-process-exporter/pkg/value"
	clientprom "github.com/prometheus/client_golang/prometheus"
	dto "github.com/prometheus/client_model/go"
)

const (
	testHelpPrometheus          = "test metric"
	testNodeNamePrometheus      = "node-a"
	testNamespacePrometheus     = "ns-a"
	testPodNamePrometheus       = "pod-a"
	testContainerNamePrometheus = "ctr-a"
	testContainerIDPrometheus   = "cid-a"
	testGpuUUIDPrometheus       = "gpu-a"
	testModelNamePrometheus     = "model-a"
)

func TestSetValueConditionalAndAddValueConditional(t *testing.T) {
	gauge := clientprom.NewGaugeVec(
		clientprom.GaugeOpts{Name: "test_conditional_gauge", Help: testHelpPrometheus},
		[]string{"label"},
	)
	counter := clientprom.NewCounterVec(
		clientprom.CounterOpts{Name: "test_conditional_counter", Help: testHelpPrometheus},
		[]string{"label"},
	)
	labels := []string{"value"}

	if ok := SetValueConditional(gauge, labels, 0, PositiveValue); ok {
		t.Fatalf("SetValueConditional returned true for zero value")
	}
	if got := gaugeValue(t, gauge.WithLabelValues(labels...)); got != 0 {
		t.Fatalf("gauge value after rejected set = %v, want 0", got)
	}
	if ok := SetValueConditional(gauge, labels, 3, PositiveValue); !ok {
		t.Fatalf("SetValueConditional returned false for positive value")
	}
	if got := gaugeValue(t, gauge.WithLabelValues(labels...)); got != 3 {
		t.Fatalf("gauge value after accepted set = %v, want 3", got)
	}

	if ok := AddValueConditional(counter, labels, 0, PositiveValue); ok {
		t.Fatalf("AddValueConditional returned true for zero value")
	}
	if got := counterValue(t, counter.WithLabelValues(labels...)); got != 0 {
		t.Fatalf("counter value after rejected add = %v, want 0", got)
	}
	if ok := AddValueConditional(counter, labels, 5, PositiveValue); !ok {
		t.Fatalf("AddValueConditional returned false for positive value")
	}
	if got := counterValue(t, counter.WithLabelValues(labels...)); got != 5 {
		t.Fatalf("counter value after accepted add = %v, want 5", got)
	}
}

func TestSetValueConditionalUnlessFingerprintedSetsZeroAfterFirstPositive(t *testing.T) {
	protectedMemoryBytes := clientprom.NewGaugeVec(
		clientprom.GaugeOpts{Name: "test_protected_memory_bytes", Help: testHelpPrometheus},
		[]string{
			LabelNode,
			LabelNamespace,
			LabelPod,
			LabelContainer,
			LabelContainerId,
			LabelGpuAllocationType,
			LabelGpuUuid,
			LabelGpuModel,
		},
	)
	gmp := &GpuMetricsProvider{
		ProtectedMemoryBytes: protectedMemoryBytes,
		fingerprintsByGauge: map[*clientprom.GaugeVec]map[uint64]bool{
			protectedMemoryBytes: make(map[uint64]bool),
		},
	}
	labels := []string{
		testNodeNamePrometheus,
		testNamespacePrometheus,
		testPodNamePrometheus,
		testContainerNamePrometheus,
		testContainerIDPrometheus,
		model.K8sResource.String(),
		testGpuUUIDPrometheus,
		testModelNamePrometheus,
	}
	fp := value.Fingerprint(labels)

	gmp.setValueConditionalUnlessFingerprinted(protectedMemoryBytes, labels, fp, 16, PositiveValue)
	if got := gaugeValue(t, protectedMemoryBytes.WithLabelValues(labels...)); got != 16 {
		t.Fatalf("first positive set = %v, want 16", got)
	}

	gmp.setValueConditionalUnlessFingerprinted(protectedMemoryBytes, labels, fp, 0, PositiveValue)
	if got := gaugeValue(t, protectedMemoryBytes.WithLabelValues(labels...)); got != 0 {
		t.Fatalf("second zero set = %v, want 0", got)
	}
}

func TestDeleteValuesClearsFingerprintsByGauge(t *testing.T) {
	containerLabels := []string{
		LabelNode,
		LabelNamespace,
		LabelPod,
		LabelContainer,
		LabelContainerId,
		LabelGpuAllocationType,
	}
	perDeviceLabels := append(append([]string{}, containerLabels...), LabelGpuUuid, LabelGpuModel)

	protectedMemoryBytes := clientprom.NewGaugeVec(
		clientprom.GaugeOpts{Name: "test_delete_protected_memory_bytes", Help: testHelpPrometheus},
		perDeviceLabels,
	)
	accountingTimeUs := clientprom.NewGaugeVec(
		clientprom.GaugeOpts{Name: "test_delete_accounting_time_us", Help: testHelpPrometheus},
		perDeviceLabels,
	)
	gmp := &GpuMetricsProvider{
		Requests:             testGaugeVec("test_requests", containerLabels),
		Limits:               testGaugeVec("test_limits", containerLabels),
		MemoryBytes:          testGaugeVec("test_memory_bytes", perDeviceLabels),
		MemoryTotalBytes:     testGaugeVec("test_memory_total_bytes", perDeviceLabels),
		MemoryFootprint:      testGaugeVec("test_memory_footprint", perDeviceLabels),
		SmUtil:               testCounterVec("test_sm_util", perDeviceLabels),
		MemoryUtil:           testCounterVec("test_memory_util", perDeviceLabels),
		EncUtil:              testCounterVec("test_enc_util", perDeviceLabels),
		DecUtil:              testCounterVec("test_dec_util", perDeviceLabels),
		OfaUtil:              testCounterVec("test_ofa_util", perDeviceLabels),
		JpgUtil:              testCounterVec("test_jpg_util", perDeviceLabels),
		ProtectedMemoryBytes: protectedMemoryBytes,
		AccountingGpuUtil:    testGaugeVec("test_accounting_gpu_util", perDeviceLabels),
		AccountingMemUtil:    testGaugeVec("test_accounting_mem_util", perDeviceLabels),
		AccountingMaxMemoryBytes: clientprom.NewGaugeVec(
			clientprom.GaugeOpts{Name: "test_accounting_max_memory_bytes", Help: testHelpPrometheus},
			perDeviceLabels,
		),
		AccountingTimeUs: accountingTimeUs,
		AccountingStartTimeUs: clientprom.NewGaugeVec(
			clientprom.GaugeOpts{Name: "test_accounting_start_time_us", Help: testHelpPrometheus},
			perDeviceLabels,
		),
		AccountingIsRunning: testGaugeVec("test_accounting_is_running", perDeviceLabels),
		fingerprintsByGauge: map[*clientprom.GaugeVec]map[uint64]bool{
			protectedMemoryBytes: make(map[uint64]bool),
			accountingTimeUs:     make(map[uint64]bool),
		},
	}
	labels := []string{
		testNodeNamePrometheus,
		testNamespacePrometheus,
		testPodNamePrometheus,
		testContainerNamePrometheus,
		testContainerIDPrometheus,
		model.K8sResource.String(),
		testGpuUUIDPrometheus,
		testModelNamePrometheus,
	}
	fp := value.Fingerprint(labels)
	gmp.fingerprintsByGauge[protectedMemoryBytes][fp] = true
	gmp.fingerprintsByGauge[accountingTimeUs][fp] = true

	gmp.DeleteValues(nil, []model.LabelFingerprint{{Labels: labels, Fingerprint: fp}})

	if gmp.fingerprintsByGauge[protectedMemoryBytes][fp] {
		t.Fatalf("fingerprint %d was not deleted", fp)
	}
	if gmp.fingerprintsByGauge[accountingTimeUs][fp] {
		t.Fatalf("second gauge fingerprint %d was not deleted", fp)
	}
}

func TestZeroValuesClearsCurrentPerDeviceGaugesOnly(t *testing.T) {
	containerLabels := []string{
		LabelNode,
		LabelNamespace,
		LabelPod,
		LabelContainer,
		LabelContainerId,
		LabelGpuAllocationType,
	}
	perDeviceLabels := append(append([]string{}, containerLabels...), LabelGpuUuid, LabelGpuModel)

	gmp := &GpuMetricsProvider{
		Requests:                 testGaugeVec("test_zero_requests", containerLabels),
		Limits:                   testGaugeVec("test_zero_limits", containerLabels),
		MemoryBytes:              testGaugeVec("test_zero_memory_bytes", perDeviceLabels),
		MemoryTotalBytes:         testGaugeVec("test_zero_memory_total_bytes", perDeviceLabels),
		MemoryFootprint:          testGaugeVec("test_zero_memory_footprint", perDeviceLabels),
		ProtectedMemoryBytes:     testGaugeVec("test_zero_protected_memory_bytes", perDeviceLabels),
		AccountingGpuUtil:        testGaugeVec("test_zero_accounting_gpu_util", perDeviceLabels),
		AccountingMemUtil:        testGaugeVec("test_zero_accounting_mem_util", perDeviceLabels),
		AccountingMaxMemoryBytes: testGaugeVec("test_zero_accounting_max_memory_bytes", perDeviceLabels),
		AccountingTimeUs:         testGaugeVec("test_zero_accounting_time_us", perDeviceLabels),
		AccountingStartTimeUs:    testGaugeVec("test_zero_accounting_start_time_us", perDeviceLabels),
		AccountingIsRunning:      testGaugeVec("test_zero_accounting_is_running", perDeviceLabels),
	}
	labels := []string{
		testNodeNamePrometheus,
		testNamespacePrometheus,
		testPodNamePrometheus,
		testContainerNamePrometheus,
		testContainerIDPrometheus,
		model.K8sResource.String(),
		testGpuUUIDPrometheus,
		testModelNamePrometheus,
	}

	SetValue(gmp.Requests, containerLabels, 1)
	SetValue(gmp.Limits, containerLabels, 2)
	SetValue(gmp.MemoryBytes, labels, 3)
	SetValue(gmp.MemoryTotalBytes, labels, 4)
	SetValue(gmp.MemoryFootprint, labels, 5)
	SetValue(gmp.ProtectedMemoryBytes, labels, 6)
	SetValue(gmp.AccountingGpuUtil, labels, 7)
	SetValue(gmp.AccountingMemUtil, labels, 8)
	SetValue(gmp.AccountingMaxMemoryBytes, labels, 9)
	SetValue(gmp.AccountingTimeUs, labels, 10)
	SetValue(gmp.AccountingStartTimeUs, labels, 11)
	SetValue(gmp.AccountingIsRunning, labels, 1)

	gmp.ZeroValues([]model.LabelFingerprint{{Labels: labels, Fingerprint: value.Fingerprint(labels)}})

	for name, metric := range map[string]clientprom.Metric{
		"MemoryBytes":          gmp.MemoryBytes.WithLabelValues(labels...),
		"MemoryFootprint":      gmp.MemoryFootprint.WithLabelValues(labels...),
		"ProtectedMemoryBytes": gmp.ProtectedMemoryBytes.WithLabelValues(labels...),
	} {
		if got := gaugeValue(t, metric); got != 0 {
			t.Fatalf("%s = %v, want 0", name, got)
		}
	}
	for name, test := range map[string]struct {
		metric clientprom.Metric
		want   float64
	}{
		"Requests":         {gmp.Requests.WithLabelValues(containerLabels...), 1},
		"Limits":           {gmp.Limits.WithLabelValues(containerLabels...), 2},
		"MemoryTotalBytes": {gmp.MemoryTotalBytes.WithLabelValues(labels...), 4},
	} {
		if got := gaugeValue(t, test.metric); got != test.want {
			t.Fatalf("%s = %v, want %v", name, got, test.want)
		}
	}
}

func TestSetMemoryValuesDoesNotResurrectZeroedContainer(t *testing.T) {
	perDeviceLabels := []string{
		LabelNode,
		LabelNamespace,
		LabelPod,
		LabelContainer,
		LabelContainerId,
		LabelGpuAllocationType,
		LabelGpuUuid,
		LabelGpuModel,
	}
	protectedMemoryBytes := clientprom.NewGaugeVec(
		clientprom.GaugeOpts{Name: "test_guarded_protected_memory_bytes", Help: testHelpPrometheus},
		perDeviceLabels,
	)
	gmp := &GpuMetricsProvider{
		MemoryBytes:          testGaugeVec("test_guarded_memory_bytes", perDeviceLabels),
		MemoryFootprint:      testGaugeVec("test_guarded_memory_footprint", perDeviceLabels),
		ProtectedMemoryBytes: protectedMemoryBytes,
		fingerprintsByGauge: map[*clientprom.GaugeVec]map[uint64]bool{
			protectedMemoryBytes: make(map[uint64]bool),
		},
	}
	labels := []string{
		testNodeNamePrometheus,
		testNamespacePrometheus,
		testPodNamePrometheus,
		testContainerNamePrometheus,
		testContainerIDPrometheus,
		model.K8sResource.String(),
		testGpuUUIDPrometheus,
		testModelNamePrometheus,
	}
	fp := value.Fingerprint(labels)

	gmp.SetMemoryValues(labels, fp, 512, 128, 25)
	gmp.ZeroValues([]model.LabelFingerprint{{Labels: labels, Fingerprint: fp}})
	gmp.SetMemoryValues(labels, fp, 256, 64, 12.5)

	for name, metric := range map[string]clientprom.Metric{
		"MemoryBytes":          gmp.MemoryBytes.WithLabelValues(labels...),
		"MemoryFootprint":      gmp.MemoryFootprint.WithLabelValues(labels...),
		"ProtectedMemoryBytes": gmp.ProtectedMemoryBytes.WithLabelValues(labels...),
	} {
		if got := gaugeValue(t, metric); got != 0 {
			t.Fatalf("%s = %v, want 0", name, got)
		}
	}
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

func testGaugeVec(name string, labels []string) *clientprom.GaugeVec {
	return clientprom.NewGaugeVec(clientprom.GaugeOpts{Name: name, Help: testHelpPrometheus}, labels)
}

func testCounterVec(name string, labels []string) *clientprom.CounterVec {
	return clientprom.NewCounterVec(clientprom.CounterOpts{Name: name, Help: testHelpPrometheus}, labels)
}
