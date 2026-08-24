// SPDX-License-Identifier: Apache-2.0

package config

import (
	"strings"
	"testing"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/value"
)

func TestValidateIntervals(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name               string
		driverPollInterval time.Duration
		scrapeInterval     time.Duration
		wantErrContains    string
	}{
		{
			name:               "valid exact multiple",
			driverPollInterval: time.Second,
			scrapeInterval:     60 * time.Second,
		},
		{
			name:               "invalid driver poll interval",
			driverPollInterval: 0,
			scrapeInterval:     60 * time.Second,
			wantErrContains:    DriverPollIntervalEnvVar,
		},
		{
			name:               "negative driver poll interval",
			driverPollInterval: -1 * time.Second,
			scrapeInterval:     60 * time.Second,
			wantErrContains:    DriverPollIntervalEnvVar,
		},
		{
			name:               "invalid scrape interval",
			driverPollInterval: time.Second,
			scrapeInterval:     0,
			wantErrContains:    ScrapeIntervalEnvVar,
		},
		{
			name:               "negative scrape interval",
			driverPollInterval: time.Second,
			scrapeInterval:     -1 * time.Second,
			wantErrContains:    ScrapeIntervalEnvVar,
		},
		{
			name:               "invalid remainder",
			driverPollInterval: 4 * time.Second,
			scrapeInterval:     10 * time.Second,
			wantErrContains:    "exact multiple",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			err := validateIntervals(tt.driverPollInterval, tt.scrapeInterval)
			if tt.wantErrContains == "" {
				if err != nil {
					t.Fatalf("validateIntervals() error = %v, want nil", err)
				}
				return
			}
			if err == nil {
				t.Fatalf("validateIntervals() error = nil, want substring %q", tt.wantErrContains)
			}
			if !strings.Contains(err.Error(), tt.wantErrContains) {
				t.Fatalf("validateIntervals() error = %q, want substring %q", err.Error(), tt.wantErrContains)
			}
		})
	}
}

func TestValidateExporterPort(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name            string
		exporterPort    uint64
		wantErrContains string
	}{
		{
			name:         "valid port",
			exporterPort: 8080,
		},
		{
			name:            "zero port",
			exporterPort:    0,
			wantErrContains: portRange,
		},
		{
			name:            "reserved port",
			exporterPort:    minAllowedPort - 1,
			wantErrContains: portRange,
		},
		{
			name:         "first allowed port",
			exporterPort: minAllowedPort,
		},
		{
			name:         "last allowed port",
			exporterPort: maxAllowedPort,
		},
		{
			name:            "port above valid range",
			exporterPort:    maxAllowedPort + 1,
			wantErrContains: portRange,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			err := validateExporterPort(tt.exporterPort)
			if tt.wantErrContains == "" {
				if err != nil {
					t.Fatalf("validateExporterPort() error = %v, want nil", err)
				}
				return
			}
			if err == nil {
				t.Fatalf("validateExporterPort() error = nil, want substring %q", tt.wantErrContains)
			}
			if !strings.Contains(err.Error(), tt.wantErrContains) {
				t.Fatalf("validateExporterPort() error = %q, want substring %q", err.Error(), tt.wantErrContains)
			}
		})
	}
}

const (
	nodeA = "node-a"
)

func TestLoadConfigReturnsErrors(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name            string
		values          map[string]string
		wantErrContains string
	}{
		{
			name:            "missing node name",
			values:          map[string]string{},
			wantErrContains: NodeNameEnvVar,
		},
		{
			name: "invalid interval",
			values: map[string]string{
				NodeNameEnvVar:           nodeA,
				DriverPollIntervalEnvVar: "4s",
				ScrapeIntervalEnvVar:     "10s",
			},
			wantErrContains: "exact multiple",
		},
		{
			name: "invalid port",
			values: map[string]string{
				NodeNameEnvVar:     nodeA,
				ExporterPortEnvVar: "65536",
			},
			wantErrContains: portRange,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			_, err := loadConfig(value.MapProvider(tt.values))
			if err == nil {
				t.Fatalf("loadConfig() error = nil, want substring %q", tt.wantErrContains)
			}
			if !strings.Contains(err.Error(), tt.wantErrContains) {
				t.Fatalf("loadConfig() error = %q, want substring %q", err.Error(), tt.wantErrContains)
			}
		})
	}
}

func TestLoadConfigSuccess(t *testing.T) {
	t.Parallel()

	cfg, err := loadConfig(value.MapProvider(map[string]string{
		NodeNameEnvVar:           nodeA,
		HostProcMountPointEnvVar: "/proc",
		DriverPollIntervalEnvVar: "2s",
		ScrapeIntervalEnvVar:     "10s",
		ExporterEndpointEnvVar:   "/custom-metrics",
		ExporterPortEnvVar:       "9443",
	}))
	if err != nil {
		t.Fatalf("loadConfig() error = %v, want nil", err)
	}
	if cfg.NodeName != nodeA ||
		cfg.HostProcMountPoint != "/proc" ||
		cfg.DriverPollInterval != 2*time.Second ||
		cfg.ScrapeInterval != 10*time.Second ||
		cfg.ExporterEndpoint != "/custom-metrics" ||
		cfg.ExporterPort != 9443 {
		t.Fatalf("loadConfig() = %+v, want configured values", cfg)
	}
}
