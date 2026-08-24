// SPDX-License-Identifier: Apache-2.0

// Package config maintains the runtime configuration of this exporter
package config

import (
	"fmt"
	"sync"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/value"
)

const (
	NodeNameEnvVar            = "NODE_NAME"
	HostProcMountPointEnvVar  = "HOST_PROC_MOUNT_POINT"
	DefaultHostProcMountPoint = "/host/proc"
	DriverPollIntervalEnvVar  = "DRIVER_POLL_INTERVAL"
	DefaultDriverPollInterval = time.Second
	ScrapeIntervalEnvVar      = "SCRAPE_INTERVAL"
	DefaultScrapeInterval     = 60 * time.Second
	ExporterEndpointEnvVar    = "EXPORTER_ENDPOINT"
	DefaultExporterEndpoint   = "/metrics"
	ExporterPortEnvVar        = "EXPORTER_PORT"
	DefaultExporterPort       = uint64(9494)
	minAllowedPort            = uint64(1024)
	maxAllowedPort            = uint64(65535)
)

type Config struct {
	NodeName           string
	HostProcMountPoint string
	DriverPollInterval time.Duration
	ScrapeInterval     time.Duration
	ExporterEndpoint   string
	ExporterPort       uint64
}

var config *Config
var configErr error

var configOnce sync.Once

func New() (*Config, error) {
	configOnce.Do(func() {
		config, configErr = loadConfig(value.EnvProvider())
	})
	return config, configErr
}

func loadConfig(provider value.Provider) (*Config, error) {
	nodeName, err := value.ParseValue[string](provider, NodeNameEnvVar)
	if err != nil {
		return nil, fmt.Errorf("parse %s: %w", NodeNameEnvVar, err)
	}
	hostProcMountPoint := value.ParseValueOrDefault[string](
		provider,
		HostProcMountPointEnvVar,
		DefaultHostProcMountPoint,
	)
	driverPollInterval := value.ParseValueOrDefault[time.Duration](
		provider,
		DriverPollIntervalEnvVar,
		DefaultDriverPollInterval,
	)
	scrapeInterval := value.ParseValueOrDefault[time.Duration](provider, ScrapeIntervalEnvVar, DefaultScrapeInterval)
	exporterEndpoint := value.ParseValueOrDefault[string](provider, ExporterEndpointEnvVar, DefaultExporterEndpoint)
	exporterPort := value.ParseValueOrDefault[uint64](provider, ExporterPortEnvVar, DefaultExporterPort)
	if err := validateIntervals(driverPollInterval, scrapeInterval); err != nil {
		return nil, fmt.Errorf("invalid config: %w", err)
	}
	if err := validateExporterPort(exporterPort); err != nil {
		return nil, fmt.Errorf("invalid config: %w", err)
	}
	return &Config{
		NodeName:           nodeName,
		HostProcMountPoint: hostProcMountPoint,
		DriverPollInterval: driverPollInterval,
		ScrapeInterval:     scrapeInterval,
		ExporterEndpoint:   exporterEndpoint,
		ExporterPort:       exporterPort,
	}, nil
}

func validateIntervals(driverPollInterval, scrapeInterval time.Duration) error {
	if driverPollInterval <= 0 {
		return fmt.Errorf("%s must be > 0, got %v", DriverPollIntervalEnvVar, driverPollInterval)
	}
	if scrapeInterval <= 0 {
		return fmt.Errorf("%s must be > 0, got %v", ScrapeIntervalEnvVar, scrapeInterval)
	}
	if scrapeInterval%driverPollInterval != 0 {
		return fmt.Errorf(
			"%s (%v) must be an exact multiple of %s (%v)",
			ScrapeIntervalEnvVar,
			scrapeInterval,
			DriverPollIntervalEnvVar,
			driverPollInterval,
		)
	}
	return nil
}

var (
	portRange = fmt.Sprintf("between %d and %d", minAllowedPort, maxAllowedPort)
)

func validateExporterPort(exporterPort uint64) error {
	if exporterPort < minAllowedPort || exporterPort > maxAllowedPort {
		return fmt.Errorf("%s must be %s, got %d", ExporterPortEnvVar, portRange, exporterPort)
	}
	return nil
}
