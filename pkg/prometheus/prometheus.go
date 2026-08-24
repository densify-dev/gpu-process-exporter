// SPDX-License-Identifier: Apache-2.0

// Package prometheus implements the exposition of metrics in Prometheus exposition format
package prometheus

import (
	"context"
	"errors"
	"net/http"
	"slices"
	"strconv"
	"sync"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"github.com/prometheus/client_golang/prometheus/promhttp"
)

const (
	LabelGpuUuid           = "gpu_uuid"
	LabelGpuModel          = "gpu_model"
	LabelNode              = "node"
	LabelNamespace         = "namespace"
	LabelPod               = "pod"
	LabelContainer         = "container"
	LabelContainerId       = "container_id"
	LabelGpuAllocationType = "gpu_allocation_type"

	MetricHelpNumberGpus             = "Number or fraction of GPUs"
	MetricHelpBytes                  = "Bytes"
	MetricHelpPercent                = "Percent"
	MetricHelpUtilizationCumulative  = "Accumulated utilization percent-seconds"
	MetricHelpMicroseconds           = "Microseconds"
	MetricHelpMicrosecondsSinceEpoch = "Microseconds since epoch"
	MetricHelpIsRunning              = "1 if still running, 0 otherwise"

	MetricRequestsName                 = "kubex_gpu_container_requests"
	MetricLimitsName                   = "kubex_gpu_container_limits"
	MetricMemoryBytesName              = "kubex_gpu_container_memory_bytes"
	MetricMemoryTotalBytesName         = "kubex_gpu_container_memory_total_bytes"
	MetricMemoryFootprintName          = "kubex_gpu_container_memory_footprint_percent"
	MetricSmUtilName                   = "kubex_gpu_container_sm_utilization_percent_seconds_total"
	MetricMemoryUtilName               = "kubex_gpu_container_memory_utilization_percent_seconds_total"
	MetricEncUtilName                  = "kubex_gpu_container_enc_utilization_percent_seconds_total"
	MetricDecUtilName                  = "kubex_gpu_container_dec_utilization_percent_seconds_total"
	MetricOfaUtilName                  = "kubex_gpu_container_ofa_utilization_percent_seconds_total"
	MetricJpgUtilName                  = "kubex_gpu_container_jpg_utilization_percent_seconds_total"
	MetricProtectedMemoryBytesName     = "kubex_gpu_container_protected_memory_bytes"
	MetricAccountingGpuUtilName        = "kubex_gpu_container_accounting_gpu_percent"
	MetricAccountingMemUtilName        = "kubex_gpu_container_accounting_memory_percent"
	MetricAccountingMaxMemoryBytesName = "kubex_gpu_container_accounting_max_memory_bytes"
	MetricAccountingTimeUsName         = "kubex_gpu_container_accounting_time_us"
	MetricAccountingStartTimeUsName    = "kubex_gpu_container_accounting_start_time_us"
	MetricAccountingIsRunningName      = "kubex_gpu_container_accounting_is_running"
)

type GpuMetricsProvider struct {
	Requests                 *prometheus.GaugeVec
	Limits                   *prometheus.GaugeVec
	MemoryBytes              *prometheus.GaugeVec
	MemoryTotalBytes         *prometheus.GaugeVec
	MemoryFootprint          *prometheus.GaugeVec
	SmUtil                   *prometheus.CounterVec
	MemoryUtil               *prometheus.CounterVec
	EncUtil                  *prometheus.CounterVec
	DecUtil                  *prometheus.CounterVec
	OfaUtil                  *prometheus.CounterVec
	JpgUtil                  *prometheus.CounterVec
	ProtectedMemoryBytes     *prometheus.GaugeVec
	AccountingGpuUtil        *prometheus.GaugeVec
	AccountingMemUtil        *prometheus.GaugeVec
	AccountingMaxMemoryBytes *prometheus.GaugeVec
	AccountingTimeUs         *prometheus.GaugeVec
	AccountingStartTimeUs    *prometheus.GaugeVec
	AccountingIsRunning      *prometheus.GaugeVec
	fingerprintsMu           sync.Mutex
	fingerprintsByGauge      map[*prometheus.GaugeVec]map[uint64]bool
	zeroedMu                 sync.Mutex
	zeroedPerDevice          map[uint64]bool
}

func NewGpuMetricsProvider() *GpuMetricsProvider {
	containerLabels := []string{
		LabelNode,
		LabelNamespace,
		LabelPod,
		LabelContainer,
		LabelContainerId,
		LabelGpuAllocationType,
	}
	perDeviceLabels := slices.Clone(containerLabels)
	perDeviceLabels = append(perDeviceLabels, LabelGpuUuid, LabelGpuModel)
	protectedMemoryBytes := newGaugeVec(MetricProtectedMemoryBytesName, MetricHelpBytes, perDeviceLabels)
	return &GpuMetricsProvider{
		Requests:                 newGaugeVec(MetricRequestsName, MetricHelpNumberGpus, containerLabels),
		Limits:                   newGaugeVec(MetricLimitsName, MetricHelpNumberGpus, containerLabels),
		MemoryBytes:              newGaugeVec(MetricMemoryBytesName, MetricHelpBytes, perDeviceLabels),
		MemoryTotalBytes:         newGaugeVec(MetricMemoryTotalBytesName, MetricHelpBytes, perDeviceLabels),
		MemoryFootprint:          newGaugeVec(MetricMemoryFootprintName, MetricHelpPercent, perDeviceLabels),
		SmUtil:                   newCounterVec(MetricSmUtilName, MetricHelpUtilizationCumulative, perDeviceLabels),
		MemoryUtil:               newCounterVec(MetricMemoryUtilName, MetricHelpUtilizationCumulative, perDeviceLabels),
		EncUtil:                  newCounterVec(MetricEncUtilName, MetricHelpUtilizationCumulative, perDeviceLabels),
		DecUtil:                  newCounterVec(MetricDecUtilName, MetricHelpUtilizationCumulative, perDeviceLabels),
		OfaUtil:                  newCounterVec(MetricOfaUtilName, MetricHelpUtilizationCumulative, perDeviceLabels),
		JpgUtil:                  newCounterVec(MetricJpgUtilName, MetricHelpUtilizationCumulative, perDeviceLabels),
		ProtectedMemoryBytes:     protectedMemoryBytes,
		AccountingGpuUtil:        newGaugeVec(MetricAccountingGpuUtilName, MetricHelpPercent, perDeviceLabels),
		AccountingMemUtil:        newGaugeVec(MetricAccountingMemUtilName, MetricHelpPercent, perDeviceLabels),
		AccountingMaxMemoryBytes: newGaugeVec(MetricAccountingMaxMemoryBytesName, MetricHelpBytes, perDeviceLabels),
		AccountingTimeUs:         newGaugeVec(MetricAccountingTimeUsName, MetricHelpMicroseconds, perDeviceLabels),
		AccountingStartTimeUs: newGaugeVec(
			MetricAccountingStartTimeUsName,
			MetricHelpMicrosecondsSinceEpoch,
			perDeviceLabels,
		),
		AccountingIsRunning: newGaugeVec(MetricAccountingIsRunningName, MetricHelpIsRunning, perDeviceLabels),
		fingerprintsByGauge: map[*prometheus.GaugeVec]map[uint64]bool{
			protectedMemoryBytes: make(map[uint64]bool),
		},
		zeroedPerDevice: make(map[uint64]bool),
	}
}

func (gmp *GpuMetricsProvider) ListenAndServe(ctx context.Context, cfg *config.Config) error {
	mux := http.NewServeMux()
	mux.Handle(cfg.ExporterEndpoint, promhttp.Handler())
	server := &http.Server{
		Addr:              ":" + strconv.FormatUint(cfg.ExporterPort, 10),
		Handler:           mux,
		ReadHeaderTimeout: 5 * time.Second,
	}

	errCh := make(chan error, 1)
	go func() {
		if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errCh <- err
			return
		}
		errCh <- nil
	}()

	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := server.Shutdown(shutdownCtx); err != nil {
			return err
		}
		return <-errCh
	case err := <-errCh:
		return err
	}
}

func (gmp *GpuMetricsProvider) DeleteValues(containerLabels [][]string, perDeviceLabels []model.LabelFingerprint) {
	for _, labels := range containerLabels {
		gmp.Requests.DeleteLabelValues(labels...)
		gmp.Limits.DeleteLabelValues(labels...)
	}
	for _, labelFingerprint := range perDeviceLabels {
		labels := labelFingerprint.Labels
		gmp.MemoryBytes.DeleteLabelValues(labels...)
		gmp.MemoryTotalBytes.DeleteLabelValues(labels...)
		gmp.MemoryFootprint.DeleteLabelValues(labels...)
		gmp.SmUtil.DeleteLabelValues(labels...)
		gmp.MemoryUtil.DeleteLabelValues(labels...)
		gmp.EncUtil.DeleteLabelValues(labels...)
		gmp.DecUtil.DeleteLabelValues(labels...)
		gmp.OfaUtil.DeleteLabelValues(labels...)
		gmp.JpgUtil.DeleteLabelValues(labels...)
		gmp.ProtectedMemoryBytes.DeleteLabelValues(labels...)
		gmp.AccountingGpuUtil.DeleteLabelValues(labels...)
		gmp.AccountingMemUtil.DeleteLabelValues(labels...)
		gmp.AccountingMaxMemoryBytes.DeleteLabelValues(labels...)
		gmp.AccountingTimeUs.DeleteLabelValues(labels...)
		gmp.AccountingStartTimeUs.DeleteLabelValues(labels...)
		gmp.AccountingIsRunning.DeleteLabelValues(labels...)
		gmp.deleteFingerprints(labelFingerprint.Fingerprint)
		gmp.deleteZeroedPerDevice(labelFingerprint.Fingerprint)
	}
}

func (gmp *GpuMetricsProvider) ZeroValues(perDeviceLabels []model.LabelFingerprint) {
	if gmp == nil {
		return
	}
	gmp.zeroedMu.Lock()
	defer gmp.zeroedMu.Unlock()
	for _, labelFingerprint := range perDeviceLabels {
		labels := labelFingerprint.Labels
		gmp.ensureZeroedPerDevice()[labelFingerprint.Fingerprint] = true
		setValueIfPresent(gmp.MemoryBytes, labels, 0.0)
		setValueIfPresent(gmp.MemoryFootprint, labels, 0.0)
		setValueIfPresent(gmp.ProtectedMemoryBytes, labels, 0.0)
	}
}

func (gmp *GpuMetricsProvider) SetMemoryValues(
	perDeviceLabels []string,
	fingerprint uint64,
	memoryBytes, protectedMemoryBytes, memoryFootprint float64,
) {
	if gmp == nil {
		return
	}
	gmp.zeroedMu.Lock()
	defer gmp.zeroedMu.Unlock()
	if gmp.zeroedPerDevice[fingerprint] {
		return
	}
	SetValue(gmp.MemoryBytes, perDeviceLabels, memoryBytes)
	gmp.setValueConditionalUnlessFingerprinted(
		gmp.ProtectedMemoryBytes,
		perDeviceLabels,
		fingerprint,
		protectedMemoryBytes,
		PositiveValue,
	)
	SetValue(gmp.MemoryFootprint, perDeviceLabels, memoryFootprint)
}

func (gmp *GpuMetricsProvider) deleteZeroedPerDevice(fingerprint uint64) {
	if gmp == nil {
		return
	}
	gmp.zeroedMu.Lock()
	defer gmp.zeroedMu.Unlock()
	delete(gmp.ensureZeroedPerDevice(), fingerprint)
}

func (gmp *GpuMetricsProvider) ensureZeroedPerDevice() map[uint64]bool {
	if gmp.zeroedPerDevice == nil {
		gmp.zeroedPerDevice = make(map[uint64]bool)
	}
	return gmp.zeroedPerDevice
}

func (gmp *GpuMetricsProvider) deleteFingerprints(fingerprint uint64) {
	if gmp == nil {
		return
	}
	gmp.fingerprintsMu.Lock()
	defer gmp.fingerprintsMu.Unlock()
	for _, fingerprints := range gmp.fingerprintsByGauge {
		delete(fingerprints, fingerprint)
	}
}

type ValueCondition func(float64) bool

func PositiveValue(value float64) bool {
	return value > 0.0
}

func SetValue(gvec *prometheus.GaugeVec, labels []string, value float64) {
	gvec.WithLabelValues(labels...).Set(value)
}

func setValueIfPresent(gvec *prometheus.GaugeVec, labels []string, value float64) {
	if gvec == nil {
		return
	}
	SetValue(gvec, labels, value)
}

func SetValueConditional(gvec *prometheus.GaugeVec, labels []string, value float64, cond ValueCondition) (ok bool) {
	if ok = cond == nil || cond(value); ok {
		SetValue(gvec, labels, value)
	}
	return
}

func (gmp *GpuMetricsProvider) setValueConditionalUnlessFingerprinted(
	gvec *prometheus.GaugeVec,
	labels []string,
	fingerprint uint64,
	value float64,
	cond ValueCondition,
) {
	var fingerprinted bool
	func() {
		gmp.fingerprintsMu.Lock()
		defer gmp.fingerprintsMu.Unlock()
		fingerprints := gmp.fingerprintsByGauge[gvec]
		if fingerprints == nil {
			return
		}
		fingerprinted = fingerprints[fingerprint]
		if !fingerprinted && (cond == nil || cond(value)) {
			fingerprints[fingerprint] = true
			fingerprinted = true
		}
	}()
	if fingerprinted {
		SetValue(gvec, labels, value)
	}
}

func AddValue(cvec *prometheus.CounterVec, labels []string, value float64) {
	cvec.WithLabelValues(labels...).Add(value)
}

func AddValueConditional(cvec *prometheus.CounterVec, labels []string, value float64, cond ValueCondition) (ok bool) {
	if ok = cond == nil || cond(value); ok {
		AddValue(cvec, labels, value)
	}
	return
}

//nolint:unparam
func newCounterVec(name, help string, labels []string) *prometheus.CounterVec {
	return promauto.NewCounterVec(prometheus.CounterOpts{Name: name, Help: help}, labels)
}

func newGaugeVec(name, help string, labels []string) *prometheus.GaugeVec {
	return promauto.NewGaugeVec(prometheus.GaugeOpts{Name: name, Help: help}, labels)
}
