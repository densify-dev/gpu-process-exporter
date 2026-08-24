// SPDX-License-Identifier: Apache-2.0

package process

import (
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"

	"github.com/densify-dev/gpu-process-exporter/pkg/config"
	"github.com/densify-dev/gpu-process-exporter/pkg/model"
	"k8s.io/apimachinery/pkg/types"
)

const (
	v2Prefix = "0::/kubepods.slice/" +
		"kubepods-besteffort.slice/" +
		"kubepods-besteffort-pod"
	kubeletPrefix = "0::/kubelet.slice/" +
		"kubelet-kubepods.slice/" +
		"kubelet-kubepods-besteffort.slice/" +
		"kubelet-kubepods-besteffort-pod"
	v1Prefix = ":/kubepods.slice/" +
		"kubepods-besteffort.slice/" +
		"kubepods-besteffort-pod"

	cgroupV2Prefix = v2Prefix
)

func path(prefix, podUID, runtime, containerID string) string {
	return prefix + podUID + ".slice/" + runtime + "-" + containerID + ".scope\n"
}

func v1(controller, podUID, runtime, containerID string) string {
	return controller + v1Prefix + podUID + ".slice/" + runtime + "-" + containerID + ".scope\n"
}

func j(parts ...string) string {
	return strings.Join(parts, "")
}

//nolint:unparam
func cgroupPath(prefix, podUID, runtime, containerID string) string {
	return path(prefix, podUID, runtime, containerID)
}

func lines(parts ...string) string {
	return j(parts...)
}

func TestParseCgroupFileExamples(t *testing.T) {
	t.Parallel()

	testCases := []struct {
		name       string
		content    string
		wantPodUID types.UID
		wantCtrID  string
	}{
		{
			name: "example_1",
			content: path(
				v2Prefix,
				j("f9fef4bd_4edf_4c73_bc07_", "7a19c6b28244"),
				"crio",
				j("898ac1424d0e9a2558e15b46a07ece3047b", "7252539fa464d3674b5121f150d9c"),
			),
			wantPodUID: types.UID("f9fef4bd-4edf-4c73-bc07-7a19c6b28244"),
			wantCtrID:  "898ac1424d0e9a2558e15b46a07ece3047b7252539fa464d3674b5121f150d9c",
		},
		{
			name: "example_2",
			content: path(
				v2Prefix,
				j("7bba7f76_c23e_409b_877c_", "74eef14879d0"),
				"crio",
				j("c8ed1847ad490a412104b2d70ee58a63f3a4", "be24ac8d1db20ca36fff325b6d70"),
			),
			wantPodUID: types.UID("7bba7f76-c23e-409b-877c-74eef14879d0"),
			wantCtrID:  "c8ed1847ad490a412104b2d70ee58a63f3a4be24ac8d1db20ca36fff325b6d70",
		},
		{
			name: "example_3",
			content: path(
				kubeletPrefix,
				j("09fe308b_db3c_43cc_9f62_", "53c828783e9a"),
				"cri-containerd",
				j("dcd72fe00f8979a60a25ca52e352548fd092b", "45255167bb8337037628e082849"),
			),
			wantPodUID: types.UID("09fe308b-db3c-43cc-9f62-53c828783e9a"),
			wantCtrID:  "dcd72fe00f8979a60a25ca52e352548fd092b45255167bb8337037628e082849",
		},
		{
			name: "example_4",
			content: v1(
				"11:pids",
				j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
				"cri-containerd",
				j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
			) +
				v1(
					"10:devices",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"9:hugetlb",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"8:memory",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"7:net_cls,net_prio",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"6:blkio",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"5:cpuset",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"4:perf_event",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"3:cpu,cpuacct",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"2:freezer",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				) +
				v1(
					"1:name=systemd",
					j("4c6aac68_ad82_4e0c_bc80_", "07a589301e24"),
					"cri-containerd",
					j("1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e", "5da"),
				),
			wantPodUID: types.UID("4c6aac68-ad82-4e0c-bc80-07a589301e24"),
			wantCtrID:  "1fcbd6ad8eba49629945c9d19ec68f23a92b952d32f9df72a25240899432e5da",
		},
		{
			name: "example_5",
			content: cgroupPath(
				cgroupV2Prefix,
				lines("d9f7737b_737b_482e_8d89_", "05752d59d625"),
				"cri-containerd",
				lines("9154ea43254f533df86c7ed0d2bb322a23264d", "880fac2ee8bf611a08410f9a0a"),
			),
			wantPodUID: types.UID("d9f7737b-737b-482e-8d89-05752d59d625"),
			wantCtrID:  "9154ea43254f533df86c7ed0d2bb322a23264d880fac2ee8bf611a08410f9a0a",
		},
		{
			name: "example_6",
			content: cgroupPath(
				cgroupV2Prefix,
				lines("bedbe9eb_d43c_4f79_9635_", "1649463ae304"),
				"cri-containerd",
				lines("867ea785b50d919131eea52a24324e405271b", "92a01377a8e05958f2610d18d3d"),
			),
			wantPodUID: types.UID("bedbe9eb-d43c-4f79-9635-1649463ae304"),
			wantCtrID:  "867ea785b50d919131eea52a24324e405271b92a01377a8e05958f2610d18d3d",
		},
		{
			name: "example_7",
			content: v1(
				"13:memory",
				"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
				"crio",
				"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
			) +
				v1(
					"12:hugetlb",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"11:blkio",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"10:perf_event",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"9:net_cls,net_prio",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"8:devices",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"7:rdma",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"6:pids",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"5:cpu,cpuacct",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"4:misc",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"3:freezer",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"2:cpuset",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				) +
				v1(
					"1:name=systemd",
					"cee29d54_4af9_4c92_8c0c_faa229ce2db6",
					"crio",
					"a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
				),
			wantPodUID: types.UID("cee29d54-4af9-4c92-8c0c-faa229ce2db6"),
			wantCtrID:  "a05dcec3727932dc85d741943963b8aa4672a3bae79d44bda3df17f1a234d029",
		},
		{
			name: "example_8",
			content: cgroupPath(
				cgroupV2Prefix,
				lines("ecc98f87_5022_4682_b30e_", "f8b1871b8265"),
				"cri-containerd",
				lines("7e756d8fe03bd506bc4ca7fc66c616a68fa47e", "5be51d1f920f8f47b9a28af1df"),
			),
			wantPodUID: types.UID("ecc98f87-5022-4682-b30e-f8b1871b8265"),
			wantCtrID:  "7e756d8fe03bd506bc4ca7fc66c616a68fa47e5be51d1f920f8f47b9a28af1df",
		},
		{
			name: "example_9",
			content: cgroupPath(
				cgroupV2Prefix,
				lines("82dcb509_5d89_4fd4_9b5d_", "64a42e23e75d"),
				"cri-containerd",
				lines("aab0d7887919da3ef2347e65d6765a9dbd1586", "02467994661ddd65b9d1140e5b"),
			),
			wantPodUID: types.UID("82dcb509-5d89-4fd4-9b5d-64a42e23e75d"),
			wantCtrID:  "aab0d7887919da3ef2347e65d6765a9dbd158602467994661ddd65b9d1140e5b",
		},
		{
			name: "example_10",
			content: cgroupPath(
				cgroupV2Prefix,
				lines("41a02ed5_2c8f_431c_8e96_", "da032c8f14ff"),
				"cri-containerd",
				lines("d7d26d07dee4f6c398f0d1bb6b3efde6f0b1e", "bb2629eb54cc62f7b8346e0a29d"),
			),
			wantPodUID: types.UID("41a02ed5-2c8f-431c-8e96-da032c8f14ff"),
			wantCtrID:  "d7d26d07dee4f6c398f0d1bb6b3efde6f0b1ebb2629eb54cc62f7b8346e0a29d",
		},
		{
			name: "example_11",
			content: cgroupPath(
				cgroupV2Prefix,
				lines("af25a378_1ca3_485e_9a39_", "7d61150490bb"),
				"cri-containerd",
				lines("db22d6fe89f8a430b65ecaa46a7d836c2516ec", "68a9c16f223506398c81bef6b2"),
			),
			wantPodUID: types.UID("af25a378-1ca3-485e-9a39-7d61150490bb"),
			wantCtrID:  "db22d6fe89f8a430b65ecaa46a7d836c2516ec68a9c16f223506398c81bef6b2",
		},
		{
			name: "example_12",
			content: lines(
				v1(
					"11:perf_event",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"10:devices",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"9:freezer",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"8:hugetlb",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"7:cpu,cpuacct",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"6:net_cls,net_prio",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"5:pids",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"4:blkio",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"3:memory",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"2:cpuset",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
				v1(
					"1:name=systemd",
					j("e2de4558_1fde_48ef_a774_", "9649d8a25bfb"),
					"cri-containerd",
					j("e726a88b0390996b59000a1ccabb9e34c1ef75", "ad765ad5dc22e3996d972f664e"),
				),
			),
			wantPodUID: types.UID("e2de4558-1fde-48ef-a774-9649d8a25bfb"),
			wantCtrID:  "e726a88b0390996b59000a1ccabb9e34c1ef75ad765ad5dc22e3996d972f664e",
		},
		{
			name: "example_13",
			content: path(
				v2Prefix,
				j("67a86498_70f2_436d_b14b_", "afb6e8562ab3"),
				"cri-containerd",
				j("32dc282d23473f4000ce1ea769da483b71d778", "66170406ee078c29030325424b"),
			),
			wantPodUID: types.UID("67a86498-70f2-436d-b14b-afb6e8562ab3"),
			wantCtrID:  "32dc282d23473f4000ce1ea769da483b71d77866170406ee078c29030325424b",
		},
		{
			name: "example_14",
			content: path(
				v2Prefix,
				j("6c1a6857_1e05_4c27_a281_", "247dadca9353"),
				"cri-containerd",
				j("e600c4be737866b758f9f1057af205c2728bb", "22354d39612bb378940e51c486d"),
			),
			wantPodUID: types.UID("6c1a6857-1e05-4c27-a281-247dadca9353"),
			wantCtrID:  "e600c4be737866b758f9f1057af205c2728bb22354d39612bb378940e51c486d",
		},
	}

	for _, tc := range testCases {
		t.Run(tc.name, func(t *testing.T) {
			t.Parallel()

			mapper := newTestMapper(t, 101, tc.content)

			got, err := mapper.parseCgroupFile(101)
			if err != nil {
				t.Fatalf("parseCgroupFile() error = %v", err)
			}
			if got.PodUid != tc.wantPodUID {
				t.Fatalf("parseCgroupFile() pod UID = %q, want %q", got.PodUid, tc.wantPodUID)
			}
			if got.ContainerId != tc.wantCtrID {
				t.Fatalf("parseCgroupFile() container ID = %q, want %q", got.ContainerId, tc.wantCtrID)
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

	t.Run("no_matching_entries", func(t *testing.T) {
		t.Parallel()

		mapper := newTestMapper(t, 102, "0::/system.slice/docker.service\n")
		if _, err := mapper.parseCgroupFile(102); err == nil {
			t.Fatal("parseCgroupFile() error = nil, want error")
		}
	})

	t.Run("multiple_pod_uids", func(t *testing.T) {
		t.Parallel()

		content := lines(
			cgroupPath(
				cgroupV2Prefix,
				lines("aaaaaaaa_aaaa_aaaa_aaaa_", "aaaaaaaaaaaa"),
				"crio",
				lines("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "bbbbbbbbbbbbbbbbbbbbbb"),
			),
			cgroupPath(
				cgroupV2Prefix,
				lines("cccccccc_cccc_cccc_cccc_", "cccccccccccc"),
				"crio",
				lines("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "bbbbbbbbbbbbbbbbbbbbbb"),
			),
		)
		mapper := newTestMapper(t, 103, content)
		if _, err := mapper.parseCgroupFile(103); err == nil {
			t.Fatal("parseCgroupFile() error = nil, want error")
		}
	})

	t.Run("multiple_container_ids", func(t *testing.T) {
		t.Parallel()

		content := lines(
			cgroupPath(
				cgroupV2Prefix,
				lines("aaaaaaaa_aaaa_aaaa_aaaa_", "aaaaaaaaaaaa"),
				"crio",
				lines("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "bbbbbbbbbbbbbbbbbbbbbb"),
			),
			cgroupPath(
				cgroupV2Prefix,
				lines("aaaaaaaa_aaaa_aaaa_aaaa_", "aaaaaaaaaaaa"),
				"crio",
				lines("cccccccccccccccccccccccccccccccccccccccc", "cccccccccccccccccccc"),
			),
		)
		mapper := newTestMapper(t, 104, content)
		if _, err := mapper.parseCgroupFile(104); err == nil {
			t.Fatal("parseCgroupFile() error = nil, want error")
		}
	})
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
