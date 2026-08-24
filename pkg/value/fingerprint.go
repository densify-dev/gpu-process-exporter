// SPDX-License-Identifier: Apache-2.0

package value

import "hash/fnv"

var delimiter = []byte{0}

func Fingerprint(strs []string) uint64 {
	h := fnv.New64a()
	for _, str := range strs {
		_, _ = h.Write([]byte(str))
		_, _ = h.Write(delimiter)
	}
	return h.Sum64()
}
