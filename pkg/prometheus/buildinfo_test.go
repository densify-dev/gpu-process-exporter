// SPDX-License-Identifier: Apache-2.0

package prometheus

import (
	"runtime"
	"testing"
)

func TestBuildInfoLabels(t *testing.T) {
	labels := BuildInfoLabels()
	if len(labels) != 5 {
		t.Fatalf("label count = %d, want 5", len(labels))
	}
	for name, want := range map[string]string{
		GoArchLabelName:    runtime.GOARCH,
		GoOsLabelName:      runtime.GOOS,
		GoVersionLabelName: runtime.Version(),
		RevisionLabelName:  revision,
		VersionLabelName:   version,
	} {
		if got := labels[name]; got != want {
			t.Errorf("%s = %q, want %q", name, got, want)
		}
	}
}

func TestBuildInfoLabelsWithBuildMetadata(t *testing.T) {
	originalVersion, originalRevision := version, revision
	t.Cleanup(func() { version, revision = originalVersion, originalRevision })
	version, revision = "v1.2.3", "0123456789abcdef0123456789abcdef01234567"
	labels := BuildInfoLabels()
	if labels[VersionLabelName] != version || labels[RevisionLabelName] != revision {
		t.Fatalf("build labels = %v, want version %q and revision %q", labels, version, revision)
	}
}
