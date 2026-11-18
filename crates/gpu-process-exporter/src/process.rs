// SPDX-License-Identifier: Apache-2.0

//! Maps host PIDs to the pod UID and container ID found in `<host proc>/<pid>/cgroup`.

use crate::model::{GpuContainerKey, Pid};
use regex::bytes::Regex;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

/// Matches `pod<UID>.slice/<runtime>-<ContainerID>.scope`, where the pod UID uses `_` or `-` separators.
static POD_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"pod([a-f0-9]{8}(?:[-_][a-f0-9]{4}){3}[-_][a-f0-9]{12})\.slice/(?:crio|cri-containerd)-([a-f0-9]{64})\.scope(?:$|[\t\n\f\r ])",
    )
    .unwrap()
});

#[derive(Default)]
struct Cache {
    pid_to_container: HashMap<Pid, (GpuContainerKey, u64)>,
    container_to_pids: HashMap<GpuContainerKey, HashSet<Pid>>,
}

impl Cache {
    fn insert(&mut self, pid: Pid, key: GpuContainerKey, inode: u64) {
        self.container_to_pids
            .entry(key.clone())
            .or_default()
            .insert(pid);
        self.pid_to_container.insert(pid, (key, inode));
    }

    fn remove_pid(&mut self, pid: Pid) {
        let Some((key, _)) = self.pid_to_container.remove(&pid) else {
            return;
        };
        if let Some(pids) = self.container_to_pids.get_mut(&key) {
            pids.remove(&pid);
            if pids.is_empty() {
                self.container_to_pids.remove(&key);
            }
        }
    }
}

pub struct Mapper {
    host_proc_mount_point: String,
    cache: RwLock<Cache>,
}

impl Mapper {
    pub fn new(host_proc_mount_point: &str) -> Self {
        Mapper {
            host_proc_mount_point: host_proc_mount_point.into(),
            cache: Default::default(),
        }
    }

    /// Returns the container key of a PID. The cache entry is valid while the inode of `<host proc>/<pid>` is
    /// unchanged; a changed inode means the PID was reused.
    pub fn get_container_key(&self, pid: Pid) -> io::Result<GpuContainerKey> {
        let cached = self
            .cache
            .read()
            .unwrap()
            .pid_to_container
            .get(&pid)
            .cloned();
        let mut inode = None;
        if let Some((key, cached_inode)) = cached {
            let current = self.process_inode(pid)?;
            if current == cached_inode {
                return Ok(key);
            }
            let mut cache = self.cache.write().unwrap();
            if cache
                .pid_to_container
                .get(&pid)
                .is_some_and(|(_, i)| *i == cached_inode)
            {
                cache.remove_pid(pid);
            }
            inode = Some(current);
        }
        let inode = match inode {
            Some(inode) => inode,
            None => self.process_inode(pid)?,
        };

        let key = self.parse_cgroup_file(pid)?;
        if self.process_inode(pid)? != inode {
            return Err(io::Error::other(format!(
                "pid {pid} changed while mapping container key"
            )));
        }

        let mut cache = self.cache.write().unwrap();
        if let Some((cached_key, cached_inode)) = cache.pid_to_container.get(&pid) {
            if *cached_inode == inode {
                return Ok(cached_key.clone());
            }
            cache.remove_pid(pid);
        }
        cache.insert(pid, key.clone(), inode);
        Ok(key)
    }

    pub fn remove_container_keys(&self, keys: &[GpuContainerKey]) {
        if keys.is_empty() {
            return;
        }
        let mut cache = self.cache.write().unwrap();
        for key in keys {
            let pids: Vec<Pid> = cache
                .container_to_pids
                .get(key)
                .into_iter()
                .flatten()
                .copied()
                .collect();
            for pid in pids {
                cache.remove_pid(pid);
            }
        }
    }

    #[cfg(test)]
    fn clear(&self) {
        *self.cache.write().unwrap() = Cache::default();
    }

    fn pid_path(&self, pid: Pid) -> PathBuf {
        PathBuf::from(format!("{}/{pid}", self.host_proc_mount_point))
    }

    fn process_inode(&self, pid: Pid) -> io::Result<u64> {
        Ok(std::fs::metadata(self.pid_path(pid))?.ino())
    }

    /// cgroup v2 has one line and v1 several; the same regex applies to both. Exactly one pod UID and one
    /// container ID must be found.
    fn parse_cgroup_file(&self, pid: Pid) -> io::Result<GpuContainerKey> {
        let data = std::fs::read(self.pid_path(pid).join("cgroup"))?;
        let mut pod_uids = HashSet::new();
        let mut container_ids = HashSet::new();
        for line in data.split(|&b| b == b'\n') {
            if let Some(caps) = POD_REGEX.captures(line.strip_suffix(b"\r").unwrap_or(line)) {
                // The captures are ASCII hex by construction.
                pod_uids.insert(String::from_utf8_lossy(&caps[1]).replace('_', "-"));
                container_ids.insert(String::from_utf8_lossy(&caps[2]).into_owned());
            }
        }
        Ok(GpuContainerKey {
            pod_uid: single(pod_uids, "pod UIDs")?,
            container_id: single(container_ids, "container IDs")?,
        })
    }
}

fn single(set: HashSet<String>, name: &str) -> io::Result<String> {
    match set.len() {
        0 => Err(io::Error::other(format!("{name} map is empty"))),
        1 => Ok(set.into_iter().next().unwrap()),
        _ => Err(io::Error::other(format!(
            "{name} map has multiple entries, expected only one"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/cgroup");

    #[derive(Deserialize)]
    struct CgroupCase {
        name: String,
        #[serde(default)]
        pod_uid: String,
        #[serde(default)]
        container_id: String,
        #[serde(default)]
        error: bool,
    }

    fn cgroup_cases(want_error: bool) -> Vec<CgroupCase> {
        let data = std::fs::read_to_string(format!("{FIXTURES}/cases.json")).unwrap();
        let all: Vec<CgroupCase> = serde_json::from_str(&data).unwrap();
        all.into_iter().filter(|c| c.error == want_error).collect()
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}.txt")).unwrap()
    }

    fn write_cgroup_file(root: &Path, pid: Pid, content: &str) {
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cgroup"), content).unwrap();
    }

    fn test_mapper(pid: Pid, content: &str) -> (tempfile::TempDir, Mapper) {
        let root = tempfile::tempdir().unwrap();
        write_cgroup_file(root.path(), pid, content);
        let mapper = Mapper::new(root.path().to_str().unwrap());
        (root, mapper)
    }

    fn content(uid: &str, runtime: &str, cid: &str) -> String {
        format!(
            "0::/kubepods.slice/kubepods-besteffort.slice/kubepods-besteffort-pod{uid}.slice/{runtime}-{cid}.scope\n"
        )
    }

    #[test]
    fn parse_cgroup_file_examples() {
        for tc in cgroup_cases(false) {
            let (_root, mapper) = test_mapper(101, &fixture(&tc.name));
            let got = mapper
                .parse_cgroup_file(101)
                .unwrap_or_else(|e| panic!("{}: {e}", tc.name));
            assert_eq!(got.pod_uid, tc.pod_uid, "{}", tc.name);
            assert_eq!(got.container_id, tc.container_id, "{}", tc.name);
        }
    }

    #[test]
    fn parse_cgroup_file_errors() {
        let (_root, mapper) = test_mapper(102, "");
        assert!(
            mapper.parse_cgroup_file(1).is_err(),
            "missing pid directory"
        );
        for tc in cgroup_cases(true) {
            let (_root, mapper) = test_mapper(102, &fixture(&tc.name));
            assert!(mapper.parse_cgroup_file(102).is_err(), "{}", tc.name);
        }
    }

    #[test]
    fn get_container_key_remove_and_clear() {
        const PID: Pid = 105;
        let (root, mapper) = test_mapper(
            PID,
            &content(
                "aaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa",
                "crio",
                &"b".repeat(64),
            ),
        );
        let first = mapper.get_container_key(PID).unwrap();

        write_cgroup_file(
            root.path(),
            PID,
            &content(
                "dddddddd_dddd_dddd_dddd_dddddddddddd",
                "crio",
                &"e".repeat(64),
            ),
        );
        assert_eq!(mapper.get_container_key(PID).unwrap(), first, "cached key");

        mapper.remove_container_keys(&[]);
        mapper.remove_container_keys(&[first]);
        assert_eq!(
            mapper.get_container_key(PID).unwrap().pod_uid,
            "dddddddd-dddd-dddd-dddd-dddddddddddd"
        );

        mapper.clear();
        let cid = "1".repeat(64);
        write_cgroup_file(
            root.path(),
            PID,
            &content(
                "ffffffff_ffff_ffff_ffff_ffffffffffff",
                "cri-containerd",
                &cid,
            ),
        );
        let cleared = mapper.get_container_key(PID).unwrap();
        assert_eq!(cleared.pod_uid, "ffffffff-ffff-ffff-ffff-ffffffffffff");
        assert_eq!(cleared.container_id, cid);
    }

    #[test]
    fn get_container_key_reloads_when_pid_directory_inode_changes() {
        const PID: Pid = 107;
        let (root, mapper) = test_mapper(
            PID,
            &content(
                "aaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa",
                "crio",
                &"b".repeat(64),
            ),
        );
        assert_eq!(
            mapper.get_container_key(PID).unwrap().pod_uid,
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
        );

        write_cgroup_file(
            root.path(),
            PID,
            &content(
                "dddddddd_dddd_dddd_dddd_dddddddddddd",
                "crio",
                &"e".repeat(64),
            ),
        );
        mapper
            .cache
            .write()
            .unwrap()
            .pid_to_container
            .get_mut(&PID)
            .unwrap()
            .1 = 0;

        let reloaded = mapper.get_container_key(PID).unwrap();
        assert_eq!(reloaded.pod_uid, "dddddddd-dddd-dddd-dddd-dddddddddddd");
        assert_eq!(reloaded.container_id, "e".repeat(64));
    }

    #[test]
    fn remove_container_keys_removes_all_pids_for_container() {
        let text = content(
            "aaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa",
            "crio",
            &"b".repeat(64),
        );
        let (root, mapper) = test_mapper(108, &text);
        write_cgroup_file(root.path(), 109, &text);
        let key_a = mapper.get_container_key(108).unwrap();
        assert_eq!(mapper.get_container_key(109).unwrap(), key_a);

        mapper.remove_container_keys(std::slice::from_ref(&key_a));

        let cache = mapper.cache.read().unwrap();
        assert!(!cache.pid_to_container.contains_key(&108));
        assert!(!cache.pid_to_container.contains_key(&109));
        assert!(!cache.container_to_pids.contains_key(&key_a));
    }

    #[test]
    fn mapper_concurrent_access() {
        const PID: Pid = 106;
        const UID: &str = "12345678-1234-1234-1234-123456789abc";
        let cid = "abcdef".repeat(10) + "abcd";
        let (_root, mapper) = test_mapper(
            PID,
            &content(
                "12345678_1234_1234_1234_123456789abc",
                "cri-containerd",
                &cid,
            ),
        );
        let mapper = Arc::new(mapper);
        let start = Arc::new(Barrier::new(6));

        let mut handles = Vec::new();
        for role in 0..6 {
            let (mapper, start) = (mapper.clone(), start.clone());
            handles.push(std::thread::spawn(move || {
                start.wait();
                for _ in 0..100 {
                    match role {
                        0..4 => assert_eq!(mapper.get_container_key(PID).unwrap().pod_uid, UID),
                        4 => {
                            let key = mapper.get_container_key(PID).unwrap();
                            mapper.remove_container_keys(&[key]);
                        }
                        _ => mapper.clear(),
                    }
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let key = mapper.get_container_key(PID).unwrap();
        assert_eq!(key.pod_uid, UID);
        assert_eq!(key.container_id, cid);
    }
}
