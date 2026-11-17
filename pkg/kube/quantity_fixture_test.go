// SPDX-License-Identifier: Apache-2.0

package kube

import (
	"encoding/json"
	"os"
	"testing"

	"k8s.io/apimachinery/pkg/api/resource"
)

// TestQuantityAsInt64Fixtures pins testdata/quantities.json to apimachinery's
// ParseQuantity and Quantity.AsInt64, which the Rust port re-implements.
func TestQuantityAsInt64Fixtures(t *testing.T) {
	data, err := os.ReadFile("../../testdata/quantities.json")
	if err != nil {
		t.Fatalf("ReadFile() error = %v", err)
	}
	var cases []struct {
		In    string `json:"in"`
		Int64 *int64 `json:"int64"`
		Error bool   `json:"error"`
	}
	if err = json.Unmarshal(data, &cases); err != nil {
		t.Fatalf("Unmarshal() error = %v", err)
	}
	for _, tc := range cases {
		q, err := resource.ParseQuantity(tc.In)
		if tc.Error {
			if err == nil {
				t.Errorf("ParseQuantity(%q) = %v, want error", tc.In, q.String())
			}
			continue
		}
		if err != nil {
			t.Errorf("ParseQuantity(%q) error = %v", tc.In, err)
			continue
		}
		got, ok := q.AsInt64()
		switch {
		case tc.Int64 == nil && ok:
			t.Errorf("AsInt64(%q) = %d, want no value", tc.In, got)
		case tc.Int64 != nil && (!ok || got != *tc.Int64):
			t.Errorf("AsInt64(%q) = %d, %v; want %d", tc.In, got, ok, *tc.Int64)
		}
	}
}
