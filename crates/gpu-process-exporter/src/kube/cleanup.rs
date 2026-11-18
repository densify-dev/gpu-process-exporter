// SPDX-License-Identifier: Apache-2.0

//! Schedules the removal of finished containers TTL after their finish time.

use crate::model::{GpuContainer, GpuContainerKey};
use jiff::Timestamp;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::time::Duration;

pub struct CleanupQueue {
    heap: BinaryHeap<Reverse<(Timestamp, GpuContainerKey)>>,
    queued: HashSet<GpuContainerKey>,
    ttl: Duration,
}

impl CleanupQueue {
    /// Containers expire 1.5 scrape intervals after they finish.
    pub fn new(scrape_interval: Duration) -> Self {
        CleanupQueue {
            heap: BinaryHeap::new(),
            queued: HashSet::new(),
            ttl: scrape_interval * 3 / 2,
        }
    }

    /// The sweep runs every half scrape interval.
    pub fn sweep_interval(scrape_interval: Duration) -> Duration {
        scrape_interval / 2
    }

    /// Queues a finished container once; running containers are ignored.
    pub fn enqueue(&mut self, container: &GpuContainer) {
        let Some(finished_at) = container.finished_at else {
            return;
        };
        if self.queued.insert(container.key.clone()) {
            self.heap
                .push(Reverse((finished_at + self.ttl, container.key.clone())));
        }
    }

    /// Pops the keys expired at `now`.
    pub fn pop_expired(&mut self, now: Timestamp) -> Vec<GpuContainerKey> {
        let mut expired = Vec::new();
        while self
            .heap
            .peek()
            .is_some_and(|Reverse((expires_at, _))| *expires_at <= now)
        {
            let Reverse((_, key)) = self.heap.pop().unwrap();
            self.queued.remove(&key);
            expired.push(key);
        }
        expired
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.queued.len()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn test_container(
        pod_uid: &str,
        container_id: &str,
        finished_at: Option<Timestamp>,
    ) -> GpuContainer {
        GpuContainer {
            key: GpuContainerKey {
                pod_uid: pod_uid.into(),
                container_id: container_id.into(),
            },
            finished_at,
            ..Default::default()
        }
    }

    #[test]
    fn cleanup_scheduler_enqueue_skips_zero_and_duplicate_containers() {
        let mut queue = CleanupQueue::new(Duration::from_millis(50));
        queue.enqueue(&test_container("running-pod", &"a".repeat(64), None));
        let finished = test_container(
            "finished-pod",
            &"b".repeat(64),
            Some(Timestamp::now() - Duration::from_secs(60)),
        );
        queue.enqueue(&finished);
        queue.enqueue(&finished);
        assert_eq!(queue.heap.len(), 1);
        assert_eq!(queue.len(), 1);
        assert!(queue.queued.contains(&finished.key));
    }
}
