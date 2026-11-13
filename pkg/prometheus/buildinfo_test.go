// SPDX-License-Identifier: Apache-2.0

package prometheus

import (
	"runtime"
	"testing"

	clientprom "github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/common/version"
)

const (
	metricBuildInfoName = buildInfoProgram + "_build_info"
)

func TestBuildInfo(t *testing.T) {
	originalVersion, originalRevision := version.Version, version.Revision
	originalRegisterer := clientprom.DefaultRegisterer
	t.Cleanup(func() {
		version.Version, version.Revision = originalVersion, originalRevision
		clientprom.DefaultRegisterer = originalRegisterer
	})

	for _, metadata := range []struct {
		name, version, revision string
	}{
		{name: "plain"},
		{name: "release", version: "v1.2.3", revision: "0123456789abcdef0123456789abcdef01234567"},
	} {
		t.Run(metadata.name, func(t *testing.T) {
			version.Version, version.Revision = metadata.version, metadata.revision
			registry := clientprom.NewRegistry()
			clientprom.DefaultRegisterer = registry
			RegisterBuildInfo()

			families, err := registry.Gather()
			if err != nil {
				t.Fatal(err)
			}
			for _, family := range families {
				if family.GetName() != metricBuildInfoName {
					continue
				}
				metrics := family.GetMetric()
				if len(metrics) != 1 || metrics[0].GetGauge().GetValue() != 1 {
					t.Fatalf("metrics = %v, want one series with value 1", metrics)
				}
				labels := map[string]string{}
				for _, label := range metrics[0].GetLabel() {
					labels[label.GetName()] = label.GetValue()
				}
				wantLabels := map[string]string{
					"version":   metadata.version,
					"revision":  version.GetRevision(),
					"branch":    version.Branch,
					"goversion": runtime.Version(),
					"goos":      runtime.GOOS,
					"goarch":    runtime.GOARCH,
					"tags":      version.GetTags(),
				}
				if len(labels) != len(wantLabels) {
					t.Fatalf("labels = %v, want %v", labels, wantLabels)
				}
				for name, want := range wantLabels {
					if got, ok := labels[name]; !ok || got != want {
						t.Errorf("label %s = %q (present: %t), want %q", name, got, ok, want)
					}
				}
				return
			}
			t.Fatalf("missing metric %s", metricBuildInfoName)
		})
	}
}
