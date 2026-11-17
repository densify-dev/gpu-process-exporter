// SPDX-License-Identifier: Apache-2.0

package process

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"sync"
	"testing"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	"k8s.io/apimachinery/pkg/types"
)

type cgroupCase struct {
	Name        string `json:"name"`
	PodUID      string `json:"pod_uid"`
	ContainerID string `json:"container_id"`
	Error       bool   `json:"error"`
}

func loadCgroupCases(t *testing.T, wantError bool) []cgroupCase {
	t.Helper()

	data, err := os.ReadFile(filepath.Join(cgroupFixturesDir, "cases.json"))
	if err != nil {
		t.Fatalf("ReadFile(cases.json) error = %v", err)
	}
	var all []cgroupCase
	if err = json.Unmarshal(data, &all); err != nil {
		t.Fatalf("Unmarshal(cases.json) error = %v", err)
	}
	var cases []cgroupCase
	for _, tc := range all {
		if tc.Error == wantError {
			cases = append(cases, tc)
		}
	}
	return cases
}

func readCgroupFixture(t *testing.T, name string) string {
	t.Helper()

	data, err := os.ReadFile(filepath.Join(cgroupFixturesDir, name+".txt"))
	if err != nil {
		t.Fatalf("ReadFile(%s) error = %v", name, err)
	}
	return string(data)
}

const cgroupFixturesDir = "../../testdata/cgroup"

func TestParseCgroupFileExamples(t *testing.T) {
	t.Parallel()

	for _, tc := range loadCgroupCases(t, false) {
		t.Run(tc.Name, func(t *testing.T) {
			t.Parallel()

			mapper := newTestMapper(t, 101, readCgroupFixture(t, tc.Name))

			got, err := mapper.parseCgroupFile(101)
			if err != nil {
				t.Fatalf("parseCgroupFile() error = %v", err)
			}
			if got.PodUid != types.UID(tc.PodUID) {
				t.Fatalf("parseCgroupFile() pod UID = %q, want %q", got.PodUid, tc.PodUID)
			}
			if got.ContainerId != tc.ContainerID {
				t.Fatalf("parseCgroupFile() container ID = %q, want %q", got.ContainerId, tc.ContainerID)
			}
		})
	}
}

func TestParseCgroupFileErrors(t *testing.T) {
	t.Parallel()

	t.Run("nil_mapper", func(t *testing.T) {
		t.Parallel()

		var mapper *Mapper
		if _, err := mapper.parseCgroupFile(1); err == nil {
			t.Fatal("parseCgroupFile() error = nil, want error")
		}
	})

	t.Run("nil_config", func(t *testing.T) {
		t.Parallel()

		mapper := &Mapper{}
		if _, err := mapper.parseCgroupFile(1); err == nil {
			t.Fatal("parseCgroupFile() error = nil, want error")
		}
	})

	for _, tc := range loadCgroupCases(t, true) {
		t.Run(tc.Name, func(t *testing.T) {
			t.Parallel()

			mapper := newTestMapper(t, 102, readCgroupFixture(t, tc.Name))
			if _, err := mapper.parseCgroupFile(102); err == nil {
				t.Fatal("parseCgroupFile() error = nil, want error")
			}
		})
	}
}

func TestGetContainerKeyRemoveAndClear(t *testing.T) {
	t.Parallel()

	const pid = model.Pid(105)

	const firstContent = "" +
		"0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod" +
		"aaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa.slice/crio-" +
		"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" +
		".scope\n"
	const secondContent = "" +
		"0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod" +
		"dddddddd_dddd_dddd_dddd_dddddddddddd.slice/crio-" +
		"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" +
		".scope\n"

	mapper := newTestMapper(t, pid, firstContent)

	firstKey, err := mapper.GetContainerKey(pid)
	if err != nil {
		t.Fatalf("GetContainerKey() first call error = %v", err)
	}

	writeCgroupFile(t, mapper.cfg.HostProcMountPoint, pid, secondContent)

	cachedKey, err := mapper.GetContainerKey(pid)
	if err != nil {
		t.Fatalf("GetContainerKey() cached call error = %v", err)
	}
	if *cachedKey != *firstKey {
		t.Fatalf("GetContainerKey() cached key = %+v, want %+v", *cachedKey, *firstKey)
	}

	mapper.RemoveContainerKeys()
	mapper.RemoveContainerKeys(nil)
	mapper.RemoveContainerKeys(firstKey)

	reloadedKey, err := mapper.GetContainerKey(pid)
	if err != nil {
		t.Fatalf("GetContainerKey() reloaded call error = %v", err)
	}
	if reloadedKey.PodUid != types.UID("dddddddd-dddd-dddd-dddd-dddddddddddd") {
		t.Fatalf(
			"GetContainerKey() reloaded pod UID = %q, want %q",
			reloadedKey.PodUid,
			"dddddddd-dddd-dddd-dddd-dddddddddddd",
		)
	}

	mapper.Clear()

	thirdContent := "" +
		"0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod" +
		"ffffffff_ffff_ffff_ffff_ffffffffffff.slice/cri-containerd-" +
		"1111111111111111111111111111111111111111111111111111111111111111" +
		".scope\n"
	writeCgroupFile(t, mapper.cfg.HostProcMountPoint, pid, thirdContent)

	clearedKey, err := mapper.GetContainerKey(pid)
	if err != nil {
		t.Fatalf("GetContainerKey() after Clear error = %v", err)
	}
	if clearedKey.PodUid != types.UID("ffffffff-ffff-ffff-ffff-ffffffffffff") {
		t.Fatalf(
			"GetContainerKey() after Clear pod UID = %q, want %q",
			clearedKey.PodUid,
			"ffffffff-ffff-ffff-ffff-ffffffffffff",
		)
	}
	if clearedKey.ContainerId != "1111111111111111111111111111111111111111111111111111111111111111" {
		t.Fatalf(
			"GetContainerKey() after Clear container ID = %q, want %q",
			clearedKey.ContainerId,
			"1111111111111111111111111111111111111111111111111111111111111111",
		)
	}
}

func TestGetContainerKeyReloadsWhenPidDirectoryInodeChanges(t *testing.T) {
	t.Parallel()

	const pid = model.Pid(107)

	firstContent := "" +
		"0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod" +
		"aaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa.slice/crio-" +
		"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" +
		".scope\n"
	secondContent := "" +
		"0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod" +
		"dddddddd_dddd_dddd_dddd_dddddddddddd.slice/crio-" +
		"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" +
		".scope\n"

	mapper := newTestMapper(t, pid, firstContent)

	firstKey, err := mapper.GetContainerKey(pid)
	if err != nil {
		t.Fatalf("GetContainerKey() first call error = %v", err)
	}
	if firstKey.PodUid != types.UID("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa") {
		t.Fatalf("GetContainerKey() first pod UID = %q", firstKey.PodUid)
	}

	writeCgroupFile(t, mapper.cfg.HostProcMountPoint, pid, secondContent)
	mapper.mu.Lock()
	cached := mapper.pidToContainer[pid]
	cached.inode = 0
	mapper.pidToContainer[pid] = cached
	mapper.mu.Unlock()

	reloadedKey, err := mapper.GetContainerKey(pid)
	if err != nil {
		t.Fatalf("GetContainerKey() after inode change error = %v", err)
	}
	if reloadedKey.PodUid != types.UID("dddddddd-dddd-dddd-dddd-dddddddddddd") {
		t.Fatalf(
			"GetContainerKey() after inode change pod UID = %q, want %q",
			reloadedKey.PodUid,
			"dddddddd-dddd-dddd-dddd-dddddddddddd",
		)
	}
	if reloadedKey.ContainerId != "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee" {
		t.Fatalf("GetContainerKey() after inode change container ID = %q", reloadedKey.ContainerId)
	}
}

func TestRemoveContainerKeysRemovesAllPidsForContainer(t *testing.T) {
	t.Parallel()

	const (
		pidA = model.Pid(108)
		pidB = model.Pid(109)
	)
	content := "" +
		"0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod" +
		"aaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa.slice/crio-" +
		"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" +
		".scope\n"

	mapper := newTestMapper(t, pidA, content)
	writeCgroupFile(t, mapper.cfg.HostProcMountPoint, pidB, content)

	keyA, err := mapper.GetContainerKey(pidA)
	if err != nil {
		t.Fatalf("GetContainerKey(%d) error = %v", pidA, err)
	}
	keyB, err := mapper.GetContainerKey(pidB)
	if err != nil {
		t.Fatalf("GetContainerKey(%d) error = %v", pidB, err)
	}
	if *keyA != *keyB {
		t.Fatalf("expected both pids to map to same container: %+v != %+v", *keyA, *keyB)
	}

	mapper.RemoveContainerKeys(keyA)

	mapper.mu.RLock()
	defer mapper.mu.RUnlock()
	if _, ok := mapper.pidToContainer[pidA]; ok {
		t.Fatalf("pid %d cache entry was not removed", pidA)
	}
	if _, ok := mapper.pidToContainer[pidB]; ok {
		t.Fatalf("pid %d cache entry was not removed", pidB)
	}
	if pids := mapper.containerToPids[*keyA]; len(pids) != 0 {
		t.Fatalf("reverse cache entry was not removed: %+v", pids)
	}
}

func TestMapperConcurrentAccess(t *testing.T) {
	t.Parallel()

	const pid = model.Pid(106)
	content := "" +
		"0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod" +
		"12345678_1234_1234_1234_123456789abc.slice/cri-containerd-" +
		"abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd" +
		".scope\n"

	mapper := newTestMapper(t, pid, content)

	var wg sync.WaitGroup
	start := make(chan struct{})

	reader := func() {
		defer wg.Done()
		<-start
		for range 100 {
			gck, err := mapper.GetContainerKey(pid)
			if err != nil {
				t.Errorf("GetContainerKey() error = %v", err)
				return
			}
			if gck.PodUid != types.UID("12345678-1234-1234-1234-123456789abc") {
				t.Errorf("GetContainerKey() pod UID = %q", gck.PodUid)
				return
			}
		}
	}

	remover := func() {
		defer wg.Done()
		<-start
		for range 100 {
			gck, err := mapper.GetContainerKey(pid)
			if err != nil {
				t.Errorf("GetContainerKey() before remove error = %v", err)
				return
			}
			mapper.RemoveContainerKeys(gck)
		}
	}

	clearer := func() {
		defer wg.Done()
		<-start
		for range 100 {
			mapper.Clear()
		}
	}

	for range 4 {
		wg.Add(1)
		go reader()
	}
	wg.Add(1)
	go remover()
	wg.Add(1)
	go clearer()

	close(start)
	wg.Wait()

	gck, err := mapper.GetContainerKey(pid)
	if err != nil {
		t.Fatalf("GetContainerKey() final error = %v", err)
	}
	if gck.PodUid != types.UID("12345678-1234-1234-1234-123456789abc") {
		t.Fatalf("GetContainerKey() final pod UID = %q", gck.PodUid)
	}
	if gck.ContainerId != "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd" {
		t.Fatalf("GetContainerKey() final container ID = %q", gck.ContainerId)
	}
}

func newTestMapper(t *testing.T, pid model.Pid, content string) *Mapper {
	t.Helper()

	root := t.TempDir()
	writeCgroupFile(t, root, pid, content)

	return &Mapper{
		cfg: &config.Config{
			HostProcMountPoint: root,
		},
	}
}

func writeCgroupFile(t *testing.T, root string, pid model.Pid, content string) {
	t.Helper()

	pidDir := filepath.Join(root, strconv.FormatUint(uint64(pid), 10))
	if err := os.MkdirAll(pidDir, 0o755); err != nil {
		t.Fatalf("MkdirAll(%q) error = %v", pidDir, err)
	}
	if err := os.WriteFile(filepath.Join(pidDir, "cgroup"), []byte(content), 0o644); err != nil {
		t.Fatalf("WriteFile(cgroup) error = %v", err)
	}
}
