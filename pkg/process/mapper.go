// SPDX-License-Identifier: Apache-2.0

// Package process implements the mapping between host PIDs to a namespace-pod-container tuple
package process

import (
	"bufio"
	"fmt"
	"os"
	"regexp"
	"strings"
	"sync"
	"syscall"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	"k8s.io/apimachinery/pkg/types"
)

type Mapper struct {
	cfg             *config.Config
	mu              sync.RWMutex
	pidToContainer  map[model.Pid]cachedContainerKey
	containerToPids map[model.GpuContainerKey]map[model.Pid]bool
}

type cachedContainerKey struct {
	key   model.GpuContainerKey
	inode uint64
}

func NewMapper(cfg *config.Config) *Mapper {
	return &Mapper{
		cfg:             cfg,
		pidToContainer:  make(map[model.Pid]cachedContainerKey),
		containerToPids: make(map[model.GpuContainerKey]map[model.Pid]bool),
	}
}

func (m *Mapper) GetContainerKey(pid model.Pid) (gck *model.GpuContainerKey, err error) {
	if m == nil {
		err = fmt.Errorf("mapper is nil")
		return
	}

	var inode uint64
	var inodeFound bool
	if gck, inode, inodeFound, err = m.getCachedContainerKey(pid); err != nil || gck != nil {
		return
	}

	if !inodeFound {
		inode, err = m.processInode(pid)
	}
	if err != nil {
		return
	}

	if gck, err = m.parseCgroupFile(pid); err != nil {
		return
	}
	if currentInode, inodeErr := m.processInode(pid); inodeErr != nil {
		err = inodeErr
		return
	} else if currentInode != inode {
		err = fmt.Errorf("pid %d changed while mapping container key", pid)
		return
	}

	m.mu.Lock()
	defer m.mu.Unlock()
	m.ensureMapsLocked()
	if cached, found := m.pidToContainer[pid]; found {
		if cached.inode == inode {
			gck = &cached.key
			return
		}
		m.removeCachedPidLocked(pid)
	}
	m.pidToContainer[pid] = cachedContainerKey{key: *gck, inode: inode}
	m.addReverseMappingLocked(*gck, pid)
	return
}

func (m *Mapper) getCachedContainerKey(
	pid model.Pid,
) (gck *model.GpuContainerKey, inode uint64, inodeFound bool, err error) {
	m.mu.RLock()
	cached, found := m.pidToContainer[pid]
	m.mu.RUnlock()
	if !found {
		return
	}

	if inode, err = m.processInode(pid); err != nil {
		return
	}
	inodeFound = true
	if inode == cached.inode {
		gck = &cached.key
		return
	}

	m.mu.Lock()
	defer m.mu.Unlock()
	if current, ok := m.pidToContainer[pid]; ok && current.inode == cached.inode {
		m.removeCachedPidLocked(pid)
	}
	return
}

func (m *Mapper) processInode(pid model.Pid) (ino uint64, err error) {
	if m == nil || m.cfg == nil {
		err = fmt.Errorf("mapper or its config is nil")
		return
	}
	var info os.FileInfo
	if info, err = os.Stat(fmt.Sprintf("%s/%d", m.cfg.HostProcMountPoint, pid)); err != nil {
		return
	}
	stat, ok := info.Sys().(*syscall.Stat_t)
	if !ok {
		err = fmt.Errorf("unexpected stat type %T", info.Sys())
		return
	}
	ino = stat.Ino
	return
}

func (m *Mapper) ensureMapsLocked() {
	if m.pidToContainer == nil {
		m.pidToContainer = make(map[model.Pid]cachedContainerKey)
	}
	if m.containerToPids == nil {
		m.containerToPids = make(map[model.GpuContainerKey]map[model.Pid]bool)
	}
}

func (m *Mapper) addReverseMappingLocked(gck model.GpuContainerKey, pid model.Pid) {
	pids := m.containerToPids[gck]
	if pids == nil {
		pids = make(map[model.Pid]bool)
		m.containerToPids[gck] = pids
	}
	pids[pid] = true
}

func (m *Mapper) removeCachedPidLocked(pid model.Pid) {
	cached, ok := m.pidToContainer[pid]
	if !ok {
		return
	}
	delete(m.pidToContainer, pid)
	if pids := m.containerToPids[cached.key]; pids != nil {
		delete(pids, pid)
		if len(pids) == 0 {
			delete(m.containerToPids, cached.key)
		}
	}
}

func (m *Mapper) RemoveContainerKeys(gcks ...*model.GpuContainerKey) {
	if m != nil && len(gcks) > 0 {
		m.mu.Lock()
		defer m.mu.Unlock()
		m.ensureMapsLocked()
		for _, gck := range gcks {
			if gck != nil {
				for pid := range m.containerToPids[*gck] {
					m.removeCachedPidLocked(pid)
				}
			}
		}
	}
}

func (m *Mapper) Clear() {
	if m != nil {
		m.mu.Lock()
		defer m.mu.Unlock()
		m.pidToContainer = make(map[model.Pid]cachedContainerKey)
		m.containerToPids = make(map[model.GpuContainerKey]map[model.Pid]bool)
	}
}

const (
	// Matches Kubernetes cgroup paths in the form:
	// pod<UID>.slice/<runtime>-<ContainerID>.scope
	// where the pod UID may use either '_' or '-' separators.
	podUidContainerIdRegex = `pod([a-f0-9]{8}(?:[-_][a-f0-9]{4}){3}` +
		`[-_][a-f0-9]{12})\.slice/(?:crio|cri-containerd)-` +
		`([a-f0-9]{64})\.scope(?:$|\s)`
)

var podRegex = regexp.MustCompile(podUidContainerIdRegex)

func (m *Mapper) parseCgroupFile(pid model.Pid) (gck *model.GpuContainerKey, err error) {
	if m == nil || m.cfg == nil {
		return nil, fmt.Errorf("mapper or its config is nil")
	}
	cgroupFilePath := fmt.Sprintf("%s/%d/cgroup", m.cfg.HostProcMountPoint, pid)
	var cgroupFile *os.File
	if cgroupFile, err = os.Open(cgroupFilePath); err != nil {
		return
	}
	defer func() {
		_ = cgroupFile.Close()
	}()
	// cgroup v2 has one line, while cgroup v1 has multiple lines;
	// in both cases we can apply the same regex to find the pod UID and container ID
	podUids := make(map[string]bool)
	containerIds := make(map[string]bool)
	scanner := bufio.NewScanner(cgroupFile)
	for scanner.Scan() {
		line := scanner.Text()
		if matches := podRegex.FindStringSubmatch(line); len(matches) >= 3 {
			podUids[strings.ReplaceAll(matches[1], "_", "-")] = true
			containerIds[matches[2]] = true
		}
	}
	if err = scanner.Err(); err != nil {
		return
	}
	if err = validateMap(podUids, "pod UIDs"); err != nil {
		return
	}
	if err = validateMap(containerIds, "container IDs"); err != nil {
		return
	}
	gck = &model.GpuContainerKey{}
	for podUid := range podUids {
		gck.PodUid = types.UID(podUid)
	}
	for containerId := range containerIds {
		gck.ContainerId = containerId
	}
	return
}

func validateMap(m map[string]bool, name string) error {
	switch len(m) {
	case 0:
		return fmt.Errorf("%s map is empty", name)
	case 1:
		return nil
	default:
		return fmt.Errorf("%s map has multiple entries, expected only one", name)
	}
}
