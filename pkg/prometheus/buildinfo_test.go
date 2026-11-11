// SPDX-License-Identifier: Apache-2.0

package prometheus

import (
	"os/exec"
	"runtime"
	"strings"
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
	} {
		if got := labels[name]; got != want {
			t.Errorf("%s = %q, want %q", name, got, want)
		}
	}
	for name, args := range map[string][]string{
		RevisionLabelName: {"rev-parse", "HEAD"},
		VersionLabelName:  {"tag", "--sort=-creatordate"},
	} {
		output, err := exec.Command("git", args...).Output()
		if err != nil {
			t.Fatalf("git %v: %v", args, err)
		}
		want := strings.TrimSpace(strings.SplitN(string(output), "\n", 2)[0])
		if got := labels[name]; got != want {
			t.Errorf("%s = %q, want %q; run go generate ./pkg/prometheus", name, got, want)
		}
	}
}
