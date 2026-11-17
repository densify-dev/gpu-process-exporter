// SPDX-License-Identifier: Apache-2.0

package value

import (
	"encoding/json"
	"os"
	"testing"
	"time"
)

// TestParseDurationFixtures pins testdata/durations.json to Go's time.ParseDuration,
// which the Rust port re-implements.
func TestParseDurationFixtures(t *testing.T) {
	data, err := os.ReadFile("../../testdata/durations.json")
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}
	var cases []struct {
		In    string `json:"in"`
		Ns    *int64 `json:"ns"`
		Error bool   `json:"error"`
	}
	if err = json.Unmarshal(data, &cases); err != nil {
		t.Fatalf("Unmarshal() error = %v", err)
	}
	for _, tc := range cases {
		got, err := time.ParseDuration(tc.In)
		if tc.Error {
			if err == nil {
				t.Errorf("ParseDuration(%q) = %v, want error", tc.In, got)
			}
			continue
		}
		if err != nil || int64(got) != *tc.Ns {
			t.Errorf("ParseDuration(%q) = %d, %v; want %d", tc.In, got, err, *tc.Ns)
		}
	}
}
