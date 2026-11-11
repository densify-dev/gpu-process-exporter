package prometheus

import (
	"runtime"

	"github.com/prometheus/client_golang/prometheus"
)

const (
	MetricBuildInfoName = "kubex_gpu_process_exporter_build_info"
	MetricHelpBuildInfo = "Build information for the GPU process exporter"
	GoArchLabelName     = "goarch"
	GoOsLabelName       = "goos"
	GoVersionLabelName  = "goversion"
	RevisionLabelName   = "revision"
	VersionLabelName    = "version"
)

var (
	version  = "dev"
	revision = "unknown"
)

func BuildInfoLabels() prometheus.Labels {
	return prometheus.Labels{
		GoArchLabelName:    runtime.GOARCH,
		GoOsLabelName:      runtime.GOOS,
		GoVersionLabelName: runtime.Version(),
		RevisionLabelName:  revision,
		VersionLabelName:   version,
	}
}
