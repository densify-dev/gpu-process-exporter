// SPDX-License-Identifier: Apache-2.0

package value

import "testing"

func TestFingerprintUsesLabelBoundaries(t *testing.T) {
	if got, want := Fingerprint([]string{"ab", "c"}), Fingerprint([]string{"a", "bc"}); got == want {
		t.Fatalf("fingerprints should differ when label boundaries differ: got %d", got)
	}
}
