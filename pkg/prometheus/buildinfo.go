package prometheus

import (
	_ "embed"
	"strings"

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

//go:generate mkdir -p gen
//go:generate sh -c "go env GOARCH > gen/goarch.txt"
//go:generate sh -c "go env GOOS > gen/goos.txt"
//go:generate sh -c "go env GOVERSION > gen/goversion.txt"
//go:generate sh -c "git rev-parse HEAD > gen/revision.txt"
//go:generate sh -c "git tag --sort=-creatordate > gen/tags.txt && head -n 1 gen/tags.txt > gen/version.txt"

//go:embed gen/goarch.txt
var goArch string

//go:embed gen/goos.txt
var goOs string

//go:embed gen/goversion.txt
var goVersion string

//go:embed gen/revision.txt
var revision string

//go:embed gen/version.txt
var version string

func BuildInfoLabels() prometheus.Labels {
	return prometheus.Labels{
		GoArchLabelName:    strings.TrimSpace(goArch),
		GoOsLabelName:      strings.TrimSpace(goOs),
		GoVersionLabelName: strings.TrimSpace(goVersion),
		RevisionLabelName:  strings.TrimSpace(revision),
		VersionLabelName:   strings.TrimSpace(version),
	}
}
