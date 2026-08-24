// SPDX-License-Identifier: Apache-2.0

package kube

import (
	"container/heap"
	"context"
	"time"

	"github.com/densify-dev/gpu-process-exporter/pkg/model"
)

type CleanupScheduler struct {
	parent         *Store
	gpuContainers  cleanupHeap
	queued         map[string]*cleanupItem
	tickerInterval time.Duration
	ttl            time.Duration
}

func NewCleanupScheduler(parent *Store) *CleanupScheduler {
	return &CleanupScheduler{
		parent:         parent,
		gpuContainers:  make(cleanupHeap, 0, 1024),
		queued:         make(map[string]*cleanupItem, 1024),
		tickerInterval: parent.cfg.ScrapeInterval / 2,
		ttl:            (parent.cfg.ScrapeInterval * 3) / 2,
	}
}

type cleanupItem struct {
	key       string
	container *model.GpuContainer
	expiresAt time.Time
	index     int
}

type cleanupHeap []*cleanupItem

func (h cleanupHeap) Len() int {
	return len(h)
}

func (h cleanupHeap) Less(i, j int) bool {
	return h[i].expiresAt.Before(h[j].expiresAt)
}

func (h cleanupHeap) Swap(i, j int) {
	h[i], h[j] = h[j], h[i]
	h[i].index = i
	h[j].index = j
}

func (h *cleanupHeap) Push(x interface{}) {
	item := x.(*cleanupItem)
	item.index = len(*h)
	*h = append(*h, item)
}

func (h *cleanupHeap) Pop() interface{} {
	old := *h
	n := len(old)
	item := old[n-1]
	old[n-1] = nil
	item.index = -1
	*h = old[:n-1]
	return item
}

func (cs *CleanupScheduler) StartSweeper(ctx context.Context) {
	if cs == nil {
		return
	}

	ticker := time.NewTicker(cs.tickerInterval)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			cs.purge()
		}
	}
}

func (cs *CleanupScheduler) purge() {
	if cs == nil || cs.parent == nil {
		return
	}

	now := time.Now()
	expired := make([]*model.GpuContainerKey, 0, 16)
	expiredContainerLabels := make([][]string, 0, 16)
	expiredPerDeviceLabels := make([]model.LabelFingerprint, 0, 16)
	func() {
		cs.parent.gpuContainersMu.Lock()
		defer cs.parent.gpuContainersMu.Unlock()

		for cs.gpuContainers.Len() > 0 {
			item := cs.gpuContainers[0]
			if item == nil || item.expiresAt.After(now) {
				break
			}

			item = heap.Pop(&cs.gpuContainers).(*cleanupItem)
			delete(cs.queued, item.key)
			if item.container == nil || item.container.GpuContainerKey == nil {
				continue
			}

			containerKey := item.container.GpuContainerKey
			m := cs.parent.gpuContainers[containerKey.PodUid]
			if existing := m[containerKey.ContainerId]; existing != item.container {
				continue
			}
			expiredContainerLabels = append(expiredContainerLabels, item.container.GetContainerLabels())
			expiredPerDeviceLabels = append(expiredPerDeviceLabels, ensureAllPerDeviceLabelFingerprints(item.container)...)
			delete(m, containerKey.ContainerId)
			if len(m) == 0 {
				delete(cs.parent.gpuContainers, containerKey.PodUid)
			}
			expired = append(expired, containerKey)
		}
	}()

	cs.parent.mapper.RemoveContainerKeys(expired...)
	cs.parent.gmp.DeleteValues(expiredContainerLabels, expiredPerDeviceLabels)
}

func (cs *CleanupScheduler) enqueue(container *model.GpuContainer) {
	if cs == nil || container == nil || container.GpuContainerKey == nil || container.FinishedAt.IsZero() {
		return
	}

	key := container.String()
	if _, ok := cs.queued[key]; ok {
		return
	}

	item := &cleanupItem{
		key:       key,
		container: container,
		expiresAt: container.FinishedAt.Add(cs.ttl),
	}
	heap.Push(&cs.gpuContainers, item)
	cs.queued[key] = item
}
