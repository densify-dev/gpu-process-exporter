// SPDX-License-Identifier: Apache-2.0

package model

const (
	MiB = 1024 * 1024
)

type DeviceInfo struct {
	Uuid      string
	ModelName string
	// TotalMemory is reported by NVML in bytes.
	TotalMemory       uint64
	AccountingEnabled bool
}
